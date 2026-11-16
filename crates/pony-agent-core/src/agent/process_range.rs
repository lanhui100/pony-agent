//! B207：托管范围 owner 抽象（signal / 静默等待 / 成员数 / 清理正交分离）。
//!
//! ## 为什么需要一层 owner
//! 进程组（pgid）的结构性上限是：`setsid` / reparent 逃逸者**收不到**信号 —— 它们已不在该组里。
//! cgroup 成员资格则**持久且可独立查询**，所以 Linux 上可以优先用 user-systemd 临时 scope
//! （`systemd-run --user --scope`）作为「托管范围」的真身；不可用时**一次性告警**并降级回
//! 现有 pgid 路径（B206 的唯一终止路径语义不变）。
//!
//! ## 默认关闭 / 零行为变更（B207 的 blast radius 收口）
//! `ProcessManager::new()` / `with_buffer_caps()` / `with_limits()` 一律是
//! [`RangeOwnership::Pid`]，因此 B206 的 5 条冻结测试与既有单测**一条都不受影响**；
//! 新行为只在显式 `ProcessManager::with_range_ownership(...)` 时启用。
//!
//! ## 本模块只做静态判定与文本解析，不做任何“猜测式清理”
//! - 信号走 `systemctl --user kill --kill-who=all`（按 cgroup 整组，不按 argv 模式猜 pid）；
//! - 清理走 `systemctl --user stop`（`--collect` 让空 unit 自动回收，stop 只是兜底）；
//! - 活跃性只认 `LoadState` / `ActiveState` / `TasksCurrent` 三字段：**拿不到成员数一律
//!   `Unavailable`（不得当成空）** —— 这是 B206 契约 §6.2「绝不谎报静止」的同一条纪律。
//!
//! ## B207b：spawn 侧接线（原生归属真正生效的那一半）
//! `ProcessManager::with_range_ownership(SystemdScope)` 且探测通过时，spawn 用
//! [`scope_wrapped_argv`] 把目标进程包进 `systemd-run --user --scope …`，因此目标进程成为
//! 一个**临时 scope 的成员**：cgroup 成员资格在组长退出后依然存在、pgid 可被复用而 cgroup 不会，
//! 这就是「原生归属」的全部价值。unit 名由 systemd 生成（argv 里**不带** `--unit`，保证 argv
//! 逐字透传），spawn 后由 [`parse_scope_unit_from_cgroup`] 从 `/proc/<pid>/cgroup` 解析并记入
//! `ManagedProcess::scope_unit`；解析口径是「**最末一个以 `.scope` 结尾的路径段**」（hybrid cgroup /
//! 嵌套 `app.slice` 下的实际形态未在本机验证过 —— fail-loud 兜住，绝不静默错管）。
//! 解析不到 ⇒ spawn `Err`（fail loud，且**禁止**回退 `Pid` 重放，因为无法证明目标没执行过）。
//!
//! ## 已知限制与后续项（B207c 候选）
//! - **B207c 候选**：改用 `--unit=<minted>`（unit 名 spawn 前即确定）可消除对 cgroup 解析的依赖、
//!   并让 [`unit_name_for`] / [`SystemdScopeOwner::for_leader`] 取得生产消费者；代价是改动已冻结的
//!   `systemd-run` argv，须在有 user manager 的宿主机就绪后单独一轮做（本轮不做，避免对一条
//!   **不可 live 验证**的路径做第二次未验证改动）。
//!
//! ## [SMOKE DEGRADED: NO-USER-SYSTEMD]（本机无法 live 验证）
//! 本开发机无 user manager（`systemctl --user show-environment` ⇒
//! `Failed to connect to bus: No medium found`），所以 **live scope 路径零实测**，当前只有五条
//! fake 腿覆盖：① 探测（注入 runner）② `systemctl show` 输出解析 ③ `systemd-run` argv 逐字透传
//! ④ 探测失败 ⇒ 一次性告警 + 降级 `Pid` ⑤ scope 不可用 ⇒ 终止路径 `Err`（fail loud）。
//! **上生产前必须在具备 user manager 的宿主机做一次 live 验证**，清单按优先级：
//! 1. **fork-vs-exec（第 1 项，最该先验）**：`--scope` 下 `systemd-run` 应**原地 `exec`**，使
//!    `child.id()` 同时是 FENCE 用的 leader pid 与 scope 的 `MainPID`。若某 systemd 版本改为
//!    **fork**，`/proc/<leader_pid>/stat` 读到的是 `systemd-run` 而非目标进程 ⇒ FENCE 的
//!    start-time 比对失败 ⇒ 判 `Stale` ⇒ **零信号投递**（表现为「kill 返回 Ok 但进程还活着」的
//!    反面：`Ok` 且真的没杀）。见到该症状先查这一条。
//! 2. 真实 `systemd-run --scope` 建 scope + `/proc/<pid>/cgroup` 里 unit 名的真实形态（是否确为
//!    最末 `.scope` 段；hybrid cgroup / 嵌套 `app.slice` 下的形态）；
//! 3. `systemctl --user kill --kill-who=all --signal=TERM|KILL` 与 `show`/`stop` 全链路、
//!    `deactivating` 的滞后窗口、`--collect` 对空 unit 的回收时机。

use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// B207：范围静默判定结论（字段口径对齐 B206 契约 §5 的 `probe=` 证据格式）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RangeQuiescence {
    /// 已证明为空（`source` 是证据名，进 `Err`/日志时复用 B206 的 `probe=` 口径）。
    Empty { source: &'static str },
    /// 非空：`members` 为已知存活成员（cgroup 侧拿不到 pid 时为空列表，非空结论由成员数证明）。
    NotEmpty { members: Vec<i32>, source: &'static str },
    /// 不可证明（≠ 空）：按契约 §6.2 必须由调用方转 `Err`，不得谎报静止。
    Unavailable { source: &'static str },
}

/// B207：一个「托管范围」的四项正交能力（发信号 / 等静默 / 查成员数 / 清理）。
///
/// 之所以把四件事拆开：B206 的 LADDER 需要「先投递、再判定」，而不同 owner 的投递与判定
/// 原语完全不同（pgid: `killpg` + `/proc` 扫描；cgroup: `systemctl kill` + `systemctl show`），
/// 混在一个函数里必然出现双口径（B206 契约 U2 明令禁止）。
pub(crate) trait RangeOwner: Send + Sync {
    /// 投递一个信号（`libc::SIGTERM` / `libc::SIGKILL`）到整个范围。
    fn signal(&self, sig: i32);

    /// 有条件轮询直到「范围为空」或到达 `deadline`，返回最后一次判定。
    fn wait_quiescent(&self, deadline: Instant) -> RangeQuiescence;

    /// cgroup 成员数（`Some(0)` 也表示「可查且为空」）；`None` = 查不到。
    fn inspect_task_count(&self) -> Option<u32>;

    /// 释放 owner 侧资源（scope / 临时文件）。必须幂等。
    fn cleanup(&self);
}

/// B207：owner 的选择开关（公开 opt-in）。默认 [`RangeOwnership::Pid`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOwnership {
    /// 现有语义：POSIX 进程组 + B206 的 PROBE-A/B（默认，零行为变更）。
    Pid,
    /// Linux cgroup 语义：user-systemd 临时 scope，可覆盖 `setsid`/reparent 逃逸者。
    SystemdScope,
}

// ── B207 / PidRangeOwner：完全复用 B206 的既有原语，绝不复制第二份逻辑 ──────────────
//
// 契约 U2 要求「发信号」「做空证明」只有一处实现。本 owner 因此**直接调用** `process.rs` 的
// `signal_range` / `wait_range_empty` / `scan_range_members`（同 crate 内 `pub(crate)`），
// 把结论映射成 owner 口径，而不是重写一份 pgid 探针。
#[cfg(not(windows))]
pub(crate) struct PidRangeOwner {
    pgid: i32,
}

#[cfg(not(windows))]
impl PidRangeOwner {
    pub(crate) fn new(pgid: i32) -> Self {
        Self { pgid }
    }

}

#[cfg(not(windows))]
impl RangeOwner for PidRangeOwner {
    fn signal(&self, sig: i32) {
        super::signal_range(self.pgid, sig);
    }

    fn wait_quiescent(&self, deadline: Instant) -> RangeQuiescence {
        let probe = super::wait_range_empty(self.pgid, deadline);
        if probe.empty {
            RangeQuiescence::Empty {
                source: probe.source,
            }
        } else {
            RangeQuiescence::NotEmpty {
                members: probe.members,
                source: probe.source,
            }
        }
    }

    /// B207：**仅供非 Pid 诊断口径的对照**——语义是「**组内非僵尸成员数**」（`/proc` 扫描），
    /// **不等价**于 cgroup 的 `TasksCurrent`（不覆盖 `setsid`/reparent 逃逸者），
    /// **不得**当作 cgroup 等价物使用；Pid 终止路径（B206）也从不调用它。
    fn inspect_task_count(&self) -> Option<u32> {
        super::scan_range_members(self.pgid).map(|members| members.len() as u32)
    }

    fn cleanup(&self) {
        // 进程组没有外部资源可释放（组内成员随信号消失）。
    }
}

// ── B207 / SystemdScopeOwner：cgroup 语义 ───────────────────────────────────────────

/// `systemd-run` 的 scope 固定前缀参数（顺序即契约，argv 逐字透传，不经 shell、不做变量展开）。
pub(crate) const SCOPE_RUN_ARGS: [&str; 5] = [
    "--user",
    "--scope",
    "--quiet",
    "--collect",
    "--expand-environment=no",
];

/// unit 名字符集：只允许 `[A-Za-z0-9_.-]`（systemd 会拒绝其它字符；静默失败比崩溃更糟）。
const UNIT_ALLOWED_EXTRA: [char; 3] = ['_', '.', '-'];
const UNIT_SUFFIX: &str = ".scope";
const UNIT_PREFIX: &str = "pony-agent-b207-";
const UNIT_MAX_LEN: usize = 255;

/// 可执行文件路径允许测试注入（fixture 假脚本），生产取 PATH 里的同名命令。
const SYSTEMD_RUN_BIN_ENV: &str = "PONY_B207_SYSTEMD_RUN_BIN";
const SYSTEMCTL_BIN_ENV: &str = "PONY_B207_SYSTEMCTL_BIN";
const B207_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// B207：确定性 unit 名 —— `pony-agent-b207-<leader_pid>-<seq>.scope`。
///
/// B207b 落地后仍**无生产消费者**：契约要求 `systemd-run` 的 argv 逐字透传（**不带** `--unit`），
/// 因此真实 unit 名由 systemd 生成、由 [`parse_scope_unit_from_cgroup`] 事后解析；
/// 本函数只在「显式 `--unit=` 部署模式」（未来可选）+ 单测里使用。
// B207c 待落地：若改用 `--unit=<minted>`（不依赖 cgroup 解析），这两处即为生产消费者。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn unit_name_for(leader_pid: i32, seq: u64) -> String {
    format!("{UNIT_PREFIX}{leader_pid}-{seq}{UNIT_SUFFIX}")
}

/// B207：unit 名合法性自检（空白 / `/` / 非 ASCII / 超长 / 缺 `.scope` 后缀一律拒绝）。
pub(crate) fn is_valid_unit_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= UNIT_MAX_LEN
        && name.ends_with(UNIT_SUFFIX)
        && name.len() > UNIT_SUFFIX.len()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || UNIT_ALLOWED_EXTRA.contains(&c))
}

/// B207：把目标程序与其参数包成 `systemd-run --user --scope … -- <program> <args…>` 的 **argv**。
///
/// 只产出 argv 列表（不执行），因此可以用单测逐字断言：args 的顺序/内容零改动，
/// 含空格、含 `$VAR` 字面量、含 `--` 自身都不受影响（`--expand-environment=no` 保证不展开）。
pub(crate) fn scope_wrapped_argv(program: &str, args: &[String]) -> Vec<String> {
    let mut argv: Vec<String> = SCOPE_RUN_ARGS.iter().map(|arg| arg.to_string()).collect();
    argv.push("--".to_string());
    argv.push(program.to_string());
    argv.extend(args.iter().cloned());
    argv
}

/// B207b：从 `/proc/<pid>/cgroup` 的内容解析出该进程所属的 **scope unit 名**。
///
/// 形态（cgroup v2，单行）：`0::/user.slice/user-1000.slice/user@1000.service/run-r1a2b.scope`。
/// 取**最末一个以 `.scope` 结尾**的路径段；拿不到 ⇒ `None`（调用方必须按 spawn `Err` 处理，
/// 绝不退回 `Pid` 重放 —— 无法证明目标没执行过）。
pub(crate) fn parse_scope_unit_from_cgroup(contents: &str) -> Option<String> {
    let mut found: Option<String> = None;
    for line in contents.lines() {
        let path = line.rsplit(':').next().unwrap_or("");
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            if segment.ends_with(".scope") && is_valid_unit_name(segment) {
                found = Some(segment.to_string());
            }
        }
    }
    found
}

/// B207：`systemctl show` 的三字段事实（缺字段 = `None`，不猜默认值）。
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ScopeFacts {
    pub load_state: Option<String>,
    pub active_state: Option<String>,
    pub tasks_current: Option<String>,
}

/// B207：解析 `systemctl show --property=…` 的 `Key=Value` 输出（键名大小写敏感，取最后一次出现）。
pub(crate) fn parse_scope_facts(stdout: &str) -> ScopeFacts {
    let mut facts = ScopeFacts::default();
    for line in stdout.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().to_string();
        match key.trim() {
            "LoadState" => facts.load_state = Some(value),
            "ActiveState" => facts.active_state = Some(value),
            "TasksCurrent" => facts.tasks_current = Some(value),
            _ => {}
        }
    }
    facts
}

/// B207：把 `systemctl show` 的输出判成 [`RangeQuiescence`]（纯函数，可单测）。
///
/// 判定口径（与 B207 派单逐条一致，顺序即优先级）：
/// 1. `LoadState=not-found` + `ActiveState=inactive`（或缺失）⇒ **已消失**（Empty）；
/// 2. `ActiveState ∈ {active, activating, reloading, deactivating}` 且 `TasksCurrent > 0` ⇒ **非空**；
///    同集合但 `TasksCurrent == 0` ⇒ 空；
/// 3. `ActiveState ∈ {inactive, failed}` ⇒ **空**；
/// 4. `TasksCurrent` 为 `[not set]` / 不可解析 / 三字段全缺 / 整段乱码 ⇒ **Unavailable**（不得当成空）。
pub(crate) fn classify_scope_state(stdout: &str) -> RangeQuiescence {
    const LIVE_STATES: [&str; 4] = ["active", "activating", "reloading", "deactivating"];
    let facts = parse_scope_facts(stdout);
    let load = facts.load_state.as_deref().unwrap_or("").trim();
    let active = facts.active_state.as_deref().unwrap_or("").trim();

    if load == "not-found" && (active == "inactive" || active.is_empty()) {
        return RangeQuiescence::Empty {
            source: "systemd-not-found",
        };
    }
    if LIVE_STATES.contains(&active) {
        return match facts
            .tasks_current
            .as_deref()
            .unwrap_or("")
            .trim()
            .parse::<u32>()
        {
            Ok(tasks) if tasks > 0 => RangeQuiescence::NotEmpty {
                // cgroup 不给 pid 列表：非空结论由 `TasksCurrent` 证明，members 留空。
                members: Vec::new(),
                source: "systemd-active-tasks",
            },
            Ok(_) => RangeQuiescence::Empty {
                source: "systemd-active-zero-tasks",
            },
            // `[not set]` / 乱码 / 字段缺失：不可证明 ≠ 空。
            Err(_) => RangeQuiescence::Unavailable {
                source: "systemd-tasks-unknown",
            },
        };
    }
    if active == "inactive" || active == "failed" {
        return RangeQuiescence::Empty {
            source: "systemd-inactive",
        };
    }
    RangeQuiescence::Unavailable {
        source: "systemd-show-unparsable",
    }
}

/// B207b：`systemd-run` 可执行文件路径（允许 `PONY_B207_SYSTEMD_RUN_BIN` 注入，spawn 侧使用）。
pub(crate) fn systemd_run_bin() -> String {
    bin_from_env(SYSTEMD_RUN_BIN_ENV, "systemd-run")
}

fn bin_from_env(var: &str, default: &str) -> String {
    std::env::var(var)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_string()
}

/// B207：以固定 argv 调用 `systemctl`（成功返回 stdout，失败返回 stderr 首行）。
fn run_systemctl(args: &[&str]) -> Result<String, String> {
    run_bin(&bin_from_env(SYSTEMCTL_BIN_ENV, "systemctl"), args)
}

/// B207：用指定可执行文件跑 `systemctl` 语义的调用（bin 可被单测替换）。
fn run_bin(bin: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|error| format!("`{bin} {}` 无法执行: {error}", args.join(" ")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "`{bin} {}` 退出码 {:?}: {}",
            args.join(" "),
            output.status.code(),
            first_line(&String::from_utf8_lossy(&output.stderr))
        ))
    }
}

/// B207：user-systemd 临时 scope 的 owner。
///
/// `unit` 为 `None` 表示**未附着**：spawn 侧尚未用 `systemd-run --scope` 包住目标进程
/// （该接线属 B207b）。此时绝不谎报静止 —— `wait_quiescent` 恒返回 [`RangeQuiescence::Unavailable`]，
/// 由调用方按契约 §6.2 转 `Err`。
#[cfg(not(windows))]
pub(crate) struct SystemdScopeOwner {
    unit: Option<String>,
    /// 仅测试注入：把 `systemctl` 换成一个**必定失败**的可执行文件，用来确定性复现
    /// 「scope 不可用 ⇒ `Unavailable` ⇒ 终止路径 `Err`」这条腿（生产为 `None`，走 env/默认）。
    systemctl_bin: Option<String>,
}

#[cfg(not(windows))]
impl SystemdScopeOwner {
    /// 用确定性 unit 名（[`unit_name_for`]）构造；非法名立即报错，绝不带病上线。
    ///
    /// 同 [`unit_name_for`]：B207b 的真实路径用 [`Self::new`]（systemd 生成的 unit 名），
    /// 本构造器服务于「显式 `--unit=` 部署模式」与单测。
    // B207c 待落地：若改用 `--unit=<minted>`（不依赖 cgroup 解析），这两处即为生产消费者。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn for_leader(leader_pid: i32, seq: u64) -> Result<Self, String> {
        Self::new(unit_name_for(leader_pid, seq))
    }

    pub(crate) fn new(unit: impl Into<String>) -> Result<Self, String> {
        let unit = unit.into();
        if !is_valid_unit_name(&unit) {
            return Err(format!(
                "B207: 非法 systemd unit 名 `{unit}`（只允许 [A-Za-z0-9_.-]、必须以 {UNIT_SUFFIX} 结尾、长度 1..={UNIT_MAX_LEN}）"
            ));
        }
        Ok(Self { unit: Some(unit), systemctl_bin: None })
    }

    /// 未附着 owner（spawn 未取得 unit 名 ⇒ 一律 `Unavailable`，绝不谎报静止）。
    pub(crate) fn unattached() -> Self {
        Self { unit: None, systemctl_bin: None }
    }

    /// 仅测试：注入一个必定失败的 `systemctl`（证明 fail-loud，不污染进程级 env）。
    #[cfg(test)]
    pub(crate) fn with_systemctl_bin(mut self, bin: impl Into<String>) -> Self {
        self.systemctl_bin = Some(bin.into());
        self
    }

    pub(crate) fn unit(&self) -> Option<&str> {
        self.unit.as_deref()
    }

    fn show_facts(&self, unit: &str) -> Result<String, String> {
        self.systemctl(&[
            "--user",
            "show",
            unit,
            "--property=LoadState",
            "--property=ActiveState",
            "--property=TasksCurrent",
        ])
    }

    /// 带 bin 覆盖的 `systemctl` 调用（生产等价于 [`run_systemctl`]）。
    fn systemctl(&self, args: &[&str]) -> Result<String, String> {
        match self.systemctl_bin.as_deref() {
            Some(bin) => run_bin(bin, args),
            None => run_systemctl(args),
        }
    }
}

#[cfg(not(windows))]
impl RangeOwner for SystemdScopeOwner {
    fn signal(&self, sig: i32) {
        let Some(unit) = self.unit() else {
            // 未附着：投递无从谈起。静默性由 `wait_quiescent` 的 `Unavailable` 负责暴露。
            return;
        };
        let signal = match sig {
            libc::SIGTERM => "TERM".to_string(),
            libc::SIGKILL => "KILL".to_string(),
            other => other.to_string(),
        };
        let _ = self.systemctl(&[
            "--user",
            "kill",
            "--kill-who=all",
            &format!("--signal={signal}"),
            unit,
        ]);
    }

    fn wait_quiescent(&self, deadline: Instant) -> RangeQuiescence {
        let Some(unit) = self.unit() else {
            return RangeQuiescence::Unavailable {
                source: "systemd-scope-unattached",
            };
        };
        loop {
            let verdict = match self.show_facts(unit) {
                Ok(stdout) => classify_scope_state(&stdout),
                Err(_) => RangeQuiescence::Unavailable {
                    source: "systemd-show-failed",
                },
            };
            if !matches!(verdict, RangeQuiescence::NotEmpty { .. }) || Instant::now() >= deadline {
                return verdict;
            }
            std::thread::sleep(B207_POLL_INTERVAL);
        }
    }

    fn inspect_task_count(&self) -> Option<u32> {
        let unit = self.unit()?;
        let stdout = self.show_facts(unit).ok()?;
        parse_scope_facts(&stdout)
            .tasks_current
            .and_then(|value| value.trim().parse::<u32>().ok())
    }

    fn cleanup(&self) {
        if let Some(unit) = self.unit() {
            let _ = self.systemctl(&["--user", "stop", unit]);
        }
    }
}

/// B207：按 owner 类型构造适配器（单一口径的 owner 工厂）。
///
/// `SystemdScope` 在 spawn 侧接线（B207b）落地前呈**未附着**态 ⇒ `wait_quiescent` 恒
/// `Unavailable` ⇒ 调用方按契约 §6.2 转 `Err`（不谎报静止）。
#[cfg(not(windows))]
pub(crate) fn owner_for(
    ownership: RangeOwnership,
    pgid: i32,
    scope_unit: Option<&str>,
) -> Box<dyn RangeOwner> {
    match ownership {
        RangeOwnership::Pid => Box::new(PidRangeOwner::new(pgid)),
        // B207b：spawn 期已把真实 unit 名记进 `ManagedProcess::scope_unit`；拿不到（或名字非法）
        // ⇒ 未附着 owner ⇒ `Unavailable` ⇒ 终止路径 `Err`（契约 §6.2：绝不谎报静止）。
        RangeOwnership::SystemdScope => match scope_unit {
            Some(unit) => SystemdScopeOwner::new(unit)
                .map(|owner| Box::new(owner) as Box<dyn RangeOwner>)
                .unwrap_or_else(|_| Box::new(SystemdScopeOwner::unattached())),
            None => Box::new(SystemdScopeOwner::unattached()),
        },
    }
}

// ── B207 / 探测与降级（一次性，禁止静默降级，禁止每次 spawn 重探）───────────────────

/// 探测用的外部命令执行器（测试注入 fixture 假脚本的 seam）。
pub(crate) type ProbeRunner<'a> = dyn Fn(&str, &[&str]) -> std::io::Result<Output> + 'a;

fn real_runner(bin: &str, args: &[&str]) -> std::io::Result<Output> {
    Command::new(bin).args(args).output()
}

/// B207：可注入的探测实现。
///
/// 判据（刻意不用 `is-system-running`：它在本机 exit 1，但 bus 也可能存在）：
/// 1. `systemd-run --version` 可执行且 exit 0；
/// 2. `systemctl --user show-environment` exit 0 ⇒ user manager 可用。
/// 已知代价（诚实声明）：探测只看「命令存在 + `show-environment` 退出码」，**不覆盖**策略层拒绝
/// （`Delegate=`/`TasksMax=`/manager 配置禁止 scope）—— 那类失败要到首次 `systemd-run --scope`
/// 才暴露。按 B207b 的硬规矩，此时 **spawn 直接 `Err`**（fail loud），**禁止**静默改用 `Pid` 重跑：
/// 无法证明目标没执行过，重放可能重复副作用。
pub(crate) fn detect_systemd_scope_support_with(runner: &ProbeRunner<'_>) -> Result<(), String> {
    let run_bin = bin_from_env(SYSTEMD_RUN_BIN_ENV, "systemd-run");
    let ctl_bin = bin_from_env(SYSTEMCTL_BIN_ENV, "systemctl");

    let version = runner(&run_bin, &["--version"])
        .map_err(|error| format!("`{run_bin} --version` 无法执行: {error}"))?;
    if !version.status.success() {
        return Err(format!(
            "`{run_bin} --version` 退出码 {:?}",
            version.status.code()
        ));
    }
    match runner(&ctl_bin, &["--user", "show-environment"]) {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "`{ctl_bin} --user show-environment` 退出码 {:?}: {}",
            output.status.code(),
            first_line(&String::from_utf8_lossy(&output.stderr))
        )),
        Err(error) => Err(format!(
            "`{ctl_bin} --user show-environment` 无法执行: {error}"
        )),
    }
}

/// B207：探测 user-systemd scope 是否可用。**每次进程只探测一次**（`OnceLock` 缓存）。
pub fn detect_systemd_scope_support() -> Result<(), String> {
    static SUPPORT: OnceLock<Result<(), String>> = OnceLock::new();
    SUPPORT
        .get_or_init(|| detect_systemd_scope_support_with(&real_runner))
        .clone()
}

static DEGRADE_WARNED: AtomicBool = AtomicBool::new(false);
static DEGRADE_WARN_COUNT: AtomicU64 = AtomicU64::new(0);
static LAST_DEGRADE_WARNING: OnceLock<String> = OnceLock::new();

/// B207：降级告警正文（**降级原因原文**必须在内，且必须点明「逃逸后代不再有保证」）。
pub(crate) fn degrade_warning_message(reason: &str) -> String {
    format!(
        "B207: range-owner 降级为 PidRangeOwner（降级原因原文: {reason}）—— setsid/reparent 逃逸的 escaped descendants 不再有保证；B206 唯一终止路径与既有 kill/shutdown 语义保持不变"
    )
}

/// B207：可注入的降级决策（纯逻辑 + 一次性告警闸门），供单测证明「warn 恰好一次」。
pub(crate) fn decide_ownership(
    requested: RangeOwnership,
    support: Result<(), String>,
    warned: &AtomicBool,
    warn: &mut dyn FnMut(&str),
) -> RangeOwnership {
    match requested {
        RangeOwnership::Pid => RangeOwnership::Pid,
        RangeOwnership::SystemdScope => match support {
            Ok(()) => RangeOwnership::SystemdScope,
            Err(reason) => {
                // 一次性：全局第一个降级者负责告警，其余静默沿用 Pid —— 但绝不「无告警降级」。
                if !warned.swap(true, Ordering::SeqCst) {
                    warn(&degrade_warning_message(&reason));
                }
                RangeOwnership::Pid
            }
        },
    }
}

/// B207：生产路径解析（探测失败 ⇒ 一次 `tracing::warn!` + 降级 `Pid`）。
pub(crate) fn resolve_range_ownership(requested: RangeOwnership) -> RangeOwnership {
    decide_ownership(
        requested,
        detect_systemd_scope_support(),
        &DEGRADE_WARNED,
        &mut |message| {
            DEGRADE_WARN_COUNT.fetch_add(1, Ordering::SeqCst);
            let _ = LAST_DEGRADE_WARNING.set(message.to_string());
            let reason = detect_systemd_scope_support().err().unwrap_or_default();
            tracing::warn!(
                target: "pony_core",
                code = "B207-range-owner-degraded",
                reason = %reason,
                "{}",
                message
            );
        },
    )
}

/// B207：最近一次降级告警原文（禁止静默降级的**机器可读收据**）。
///
/// 与 `tracing::warn!` 里的正文逐字相同，供上层观测/验收断言使用；`None` = 本进程从未降级。
pub fn last_degrade_warning() -> Option<&'static str> {
    LAST_DEGRADE_WARNING.get().map(String::as_str)
}

/// B207：降级告警累计次数（单测断言「恰好一次」）。
#[cfg(test)]
pub(crate) fn degrade_warn_count() -> u64 {
    DEGRADE_WARN_COUNT.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// B207 §1：unit 名合法性（systemd 会拒绝非法字符，静默失败比崩溃更糟 ⇒ 构造期就拒）。
    #[test]
    fn b207_unit_name_validation() {
        assert!(is_valid_unit_name(&unit_name_for(1234, 7)));
        assert_eq!(unit_name_for(1234, 7), "pony-agent-b207-1234-7.scope");
        assert!(is_valid_unit_name("a.scope"));
        assert!(is_valid_unit_name("pony-agent-b207-9-1.scope"));

        for bad in [
            "",
            "no-suffix",
            ".scope",
            "pony agent.scope",            // 空格
            "pony/agent.scope",            // 路径分隔符
            "pony-agent-b207-中文.scope",  // 非 ASCII
            "pony\tagent.scope",           // 制表符
        ] {
            assert!(!is_valid_unit_name(bad), "必须拒绝非法 unit 名: {bad:?}");
        }
        let too_long = format!("{}{}", "a".repeat(260), UNIT_SUFFIX);
        assert!(!is_valid_unit_name(&too_long), "超长名必须拒绝");

        // 构造器同样拒绝（非法名不得带病上线）；合法名可构造。
        assert!(SystemdScopeOwner::new("bad name.scope").is_err());
        assert_eq!(
            SystemdScopeOwner::for_leader(1234, 7)
                .expect("确定性 unit 名必须合法")
                .unit(),
            Some(unit_name_for(1234, 7).as_str())
        );
        assert_eq!(
            SystemdScopeOwner::new("pony-agent-b207-1-1.scope")
                .expect("合法名")
                .unit(),
            Some("pony-agent-b207-1-1.scope")
        );
    }

    /// B207 §2：`systemctl show` 输出解析 —— 四种判定，且 `[not set]` **不得判空**。
    #[test]
    fn b207_systemctl_show_parsing() {
        let active = classify_scope_state("LoadState=loaded\nActiveState=active\nTasksCurrent=3\n");
        assert_eq!(
            active,
            RangeQuiescence::NotEmpty {
                members: Vec::new(),
                source: "systemd-active-tasks"
            }
        );

        let gone = classify_scope_state("LoadState=not-found\nActiveState=inactive\n");
        assert_eq!(
            gone,
            RangeQuiescence::Empty {
                source: "systemd-not-found"
            }
        );

        let unknown_tasks = classify_scope_state("LoadState=loaded\nActiveState=active\nTasksCurrent=[not set]\n");
        assert_eq!(
            unknown_tasks,
            RangeQuiescence::Unavailable {
                source: "systemd-tasks-unknown"
            },
            "`[not set]` 是不可证明，绝不得当成空"
        );

        assert_eq!(
            classify_scope_state(""),
            RangeQuiescence::Unavailable {
                source: "systemd-show-unparsable"
            },
            "空输出不得当成空范围"
        );
        assert_eq!(
            classify_scope_state("\u{0}\u{1}garbage without keys\n"),
            RangeQuiescence::Unavailable {
                source: "systemd-show-unparsable"
            }
        );

        // 终态（failed/inactive）⇒ 空；active 但 TasksCurrent=0 ⇒ 空（可查且为 0）。
        assert_eq!(
            classify_scope_state("LoadState=loaded\nActiveState=failed\nTasksCurrent=0\n"),
            RangeQuiescence::Empty {
                source: "systemd-inactive"
            }
        );
        assert_eq!(
            classify_scope_state("LoadState=loaded\nActiveState=active\nTasksCurrent=0\n"),
            RangeQuiescence::Empty {
                source: "systemd-active-zero-tasks"
            }
        );

        // 未附着 owner：一律 Unavailable（spawn 未接线时绝不谎报静止）。
        assert_eq!(
            SystemdScopeOwner::unattached().wait_quiescent(Instant::now()),
            RangeQuiescence::Unavailable {
                source: "systemd-scope-unattached"
            }
        );
        assert_eq!(SystemdScopeOwner::unattached().inspect_task_count(), None);
    }

    /// B207b：从 `/proc/<pid>/cgroup` 解析 scope unit 名（live 路径无法在本机实测，故纯函数覆盖）。
    #[test]
    fn b207b_cgroup_scope_unit_parsing() {
        assert_eq!(
            parse_scope_unit_from_cgroup(
                "0::/user.slice/user-1000.slice/user@1000.service/run-r1a2b3c4d5e6f.scope\n"
            )
            .as_deref(),
            Some("run-r1a2b3c4d5e6f.scope")
        );
        assert_eq!(
            parse_scope_unit_from_cgroup(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/pony-agent-b207-1-1.scope\n"
            )
            .as_deref(),
            Some("pony-agent-b207-1-1.scope")
        );
        // 无 scope 段 / 空 / 非 cgroup 形态 ⇒ None（调用方必须按 spawn Err 处理）。
        assert_eq!(parse_scope_unit_from_cgroup("0::/user.slice/user-1000.slice/session-3.scope\n").as_deref(), Some("session-3.scope"));
        assert_eq!(parse_scope_unit_from_cgroup("0::/user.slice/user-1000.slice/user@1000.service/app.slice\n"), None);
        assert_eq!(parse_scope_unit_from_cgroup(""), None);
        assert_eq!(parse_scope_unit_from_cgroup("garbage"), None);
    }

    /// B207 §3：`systemd-run` argv 逐字透传（空格、`$VAR` 字面量、`--` 自身都不许被改写）。
    #[test]
    fn b207_scope_argv_passthrough() {
        let program = "sh";
        let args: Vec<String> = ["-c", "echo hi $HOME", "--", "a b", ""]
            .iter()
            .map(|value| value.to_string())
            .collect();
        let argv = scope_wrapped_argv(program, &args);
        assert_eq!(
            argv,
            vec![
                "--user".to_string(),
                "--scope".to_string(),
                "--quiet".to_string(),
                "--collect".to_string(),
                "--expand-environment=no".to_string(),
                "--".to_string(),
                "sh".to_string(),
                "-c".to_string(),
                "echo hi $HOME".to_string(),
                "--".to_string(),
                "a b".to_string(),
                "".to_string(),
            ]
        );
        // 顺序与内容零改动（逐位相等已覆盖；这里再显式断言尾部一一对应）。
        assert_eq!(&argv[7..], &args[..]);
    }

    /// B207 §4：探测失败 ⇒ `Err` + 降级 `Pid`，且 warn **恰好一次**（多次解析不重复告警）。
    #[test]
    fn b207_degrade_on_probe_failure() {
        // 假 runner：systemd-run --version 成功，show-environment 失败（模拟无 user manager）。
        let failing = |bin: &str, args: &[&str]| -> std::io::Result<Output> {
            assert!(
                bin == "systemd-run" || bin == "systemctl" || bin.contains("systemd"),
                "runner 只应收到 systemd / systemctl 系命令: {bin}"
            );
            if args == ["--version"] {
                Ok(synthetic_output(0, "systemd 255\n", ""))
            } else {
                Ok(synthetic_output(
                    1,
                    "",
                    "Failed to connect to bus: No medium found\n",
                ))
            }
        };
        let support = detect_systemd_scope_support_with(&failing);
        let reason = support.expect_err("show-environment 失败必须报 Err");
        assert!(
            reason.contains("Failed to connect to bus: No medium found"),
            "降级原因必须保留原文: {reason}"
        );

        // 可注入 seam：3 次解析只允许 1 次告警，且结果一律 Pid。
        let warned = AtomicBool::new(false);
        let mut count = 0usize;
        let mut messages: Vec<String> = Vec::new();
        for _ in 0..3 {
            let decided = decide_ownership(
                RangeOwnership::SystemdScope,
                Err(reason.clone()),
                &warned,
                &mut |message| {
                    count += 1;
                    messages.push(message.to_string());
                },
            );
            assert_eq!(decided, RangeOwnership::Pid, "探测失败必须降级为 Pid");
        }
        assert_eq!(count, 1, "降级告警必须恰好一次（不得每次 spawn 重探/重报）");
        assert!(messages[0].contains("No medium found"));
        assert!(messages[0].contains("escaped descendants 不再有保证"));

        // 探测成功 + 显式请求 ⇒ 保留 SystemdScope；默认请求 ⇒ 永远 Pid（零行为变更）。
        let ok = |_bin: &str, _args: &[&str]| -> std::io::Result<Output> {
            Ok(synthetic_output(0, "ok\n", ""))
        };
        assert!(detect_systemd_scope_support_with(&ok).is_ok());
        let mut never = |_message: &str| panic!("探测成功时不得告警");
        assert_eq!(
            decide_ownership(
                RangeOwnership::SystemdScope,
                Ok(()),
                &AtomicBool::new(false),
                &mut never
            ),
            RangeOwnership::SystemdScope
        );
        assert_eq!(
            decide_ownership(
                RangeOwnership::Pid,
                Err("boom".to_string()),
                &AtomicBool::new(false),
                &mut never
            ),
            RangeOwnership::Pid,
            "默认 Pid 不探测、不告警、零行为变更"
        );
    }

    /// B207b 裁决 #6：**陷阱已消除** —— scope 明确不可用 ⇒ `Unavailable` ⇒ 上游终止路径 `Err`，
    /// **不是**静默 `Ok`。这是「有 user manager 的宿主机上每次 kill 都返回 Ok 却啥也没杀」的反面。
    #[test]
    fn b207_scope_unavailable_is_fail_loud() {
        // 注入一个必定失败的 systemctl（路径不存在 ⇒ Command::output() 直接 Err），
        // 从而确定性复现「scope 不可用」，不依赖本机无 bus 这一偶发事实。
        let owner = SystemdScopeOwner::new("pony-agent-b207-42-1.scope")
            .expect("合法 unit 名")
            .with_systemctl_bin("/nonexistent/b207-fake-systemctl");
        let verdict = owner.wait_quiescent(Instant::now());
        assert_eq!(
            verdict,
            RangeQuiescence::Unavailable {
                source: "systemd-show-failed"
            },
            "systemctl 不可用时必须判 Unavailable（绝不当成空）"
        );
        assert_eq!(owner.inspect_task_count(), None, "拿不到成员数必须是 None");

        // 上游（B206 唯一终止路径）对该结论的处置：`probe_via_owner` ⇒ empty=false ⇒
        // 阶梯耗尽后必然走到 `range-not-quiescent` 的 `Err` 分支，而不是 `Ok`。
        let probe = super::super::probe_via_owner(&owner, Instant::now());
        assert!(!probe.empty, "Unavailable 不得被映射成 empty");
        assert_eq!(probe.source, "systemd-show-failed");

        // spawn 未取得 unit 名（或名字非法）时同样是 Unavailable ⇒ Err（不退回 Pid 重放）。
        for missing in [None, Some("bad name.scope")] {
            let fallback = super::owner_for(RangeOwnership::SystemdScope, 7, missing);
            assert_eq!(
                fallback.wait_quiescent(Instant::now()),
                RangeQuiescence::Unavailable {
                    source: "systemd-scope-unattached"
                }
            );
            assert!(!super::super::probe_via_owner(fallback.as_ref(), Instant::now()).empty);
        }

        // 对照：Pid owner 在同样的“注入失败”语境下仍按 B206 的 pgid 语义工作（不受影响）。
        let pid_owner = super::owner_for(RangeOwnership::Pid, 7, None);
        assert!(matches!(
            pid_owner.wait_quiescent(Instant::now()),
            RangeQuiescence::Empty { .. } | RangeQuiescence::NotEmpty { .. }
        ));
    }

    /// B207 §5：**本机真实降级验收**（物理收据）。
    ///
    /// 本机无 user manager（`systemctl --user show-environment` ⇒
    /// `Failed to connect to bus: No medium found`），故真实探测必然失败：
    /// `with_range_ownership(SystemdScope)` 必须降级为 `Pid` 并留下**一次**告警原文。
    #[test]
    fn b207_degrade_on_this_host() {
        // [SMOKE DEGRADED: NO-USER-SYSTEMD] 本机无 user manager，live scope 路径不可验，
        // 本用例只验「探测 → 一次性告警 → 降级 Pid」这条腿（另一条腿见 fake runner 用例）。
        let support = detect_systemd_scope_support();
        println!("B207 host probe = {support:?}");

        let manager = crate::agent::process::ProcessManager::with_range_ownership(
            RangeOwnership::SystemdScope,
        );
        let effective = manager.range_ownership();
        println!("B207 effective range ownership = {effective:?}");

        match support {
            Err(reason) => {
                assert_eq!(
                    effective,
                    RangeOwnership::Pid,
                    "无 user manager 的宿主机必须降级为 Pid"
                );
                let warning =
                    last_degrade_warning().expect("降级必须留下告警原文（禁止静默降级）");
                println!("B207 degrade warning = {warning}");
                assert!(
                    warning.contains(&reason),
                    "告警必须含降级原因原文: {warning} vs {reason}"
                );
                assert!(warning.contains("escaped descendants 不再有保证"));
                assert_eq!(
                    degrade_warn_count(),
                    1,
                    "同一进程内降级告警恰好一次（不得每次 spawn 重探）"
                );
                // 再构造一次：仍 Pid，但不重复告警。
                let again = crate::agent::process::ProcessManager::with_range_ownership(
                    RangeOwnership::SystemdScope,
                );
                assert_eq!(again.range_ownership(), RangeOwnership::Pid);
                assert_eq!(degrade_warn_count(), 1);
            }
            Ok(()) => {
                // 有 user manager 的宿主机：探测成功则保留 SystemdScope（spawn 接线属 B207b）。
                assert_eq!(effective, RangeOwnership::SystemdScope);
                println!("B207 host probe ok —— 本机具备 user systemd，未走降级分支");
            }
        }

        // 默认构造永远 Pid（B206 零行为变更）。
        assert_eq!(
            crate::agent::process::ProcessManager::new().range_ownership(),
            RangeOwnership::Pid
        );
    }

    fn synthetic_output(code: i32, stdout: &str, stderr: &str) -> Output {
        // 用 `sh -c` 造一个真实 Output：避免依赖 `Output` 的私有字段/平台细节。
        Command::new("sh")
            .arg("-c")
            .arg(format!(
                "printf '%s' '{}'; printf '%s' '{}' >&2; exit {code}",
                stdout.replace('\'', ""),
                stderr.replace('\'', "")
            ))
            .output()
            .expect("synthetic output")
    }
}
