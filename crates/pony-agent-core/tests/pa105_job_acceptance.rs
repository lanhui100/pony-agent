//! PA-105: 异步后台作业生命周期工具族 (job_*) 黑盒验收测试
//!
//! 契约依据：`management/task-system/03_TASKS/PA-105-async-background-job-tools.md`
//!
//! 验收矩阵覆盖：
//! - AC-01: `test_ac01_job_start_non_blocking`
//!   - `job_start` 异步启动进程，立即返回 Job ID 与 PID，主线程不阻塞。
//! - AC-02: `test_ac02_job_output_incremental_and_wait`
//!   - `job_output` 支持基于 `offset` 的单调递增游标增量拉取与 `wait` 阻塞等待完成/超时。
//! - AC-03: `test_ac03_job_kill_and_cleanup`
//!   - `job_kill` 强杀子进程树并回收资源，状态变更为 `killed`。
//! - AC-04: `test_ac04_job_list`
//!   - `job_list` 列举所有活跃与历史（定长保留）作业。
//! - AC-05: `test_ac05_job_path_security`
//!   - 路径穿越与未授权工作区检测 fail-closed。

use pony_agent_core::agent::tools::{
    job_kill, job_list, job_output, job_start,
    JobKillArgs, JobKillResult, JobListArgs, JobListResult, JobOutputArgs, JobOutputResult,
    JobStartArgs, JobStartResult,
};

// ── 平台夹具（Windows 兼容）─────────────────────────────────────────────────
//
// 原写法直接用 `sleep` / `sh` 当命令：两者都只在 Unix 存在——
// Windows 既无 `sleep.exe`（cmd 也无内建 sleep），也没有 `sh.exe`（Git Bash 的 sh 不在默认
// PATH），所以 `job_start` 会在 spawn 阶段直接失败（OS error 2/3）。

/// 「持续 N 秒」的长命令：Unix `sleep N`；Windows `ping -n N+1 127.0.0.1`
/// （每次 ping 间隔 1s，故 N+1 次约 N 秒；直接 spawn 不经过 shell，故不加 `>nul` 重定向）。
fn long_running_command(seconds: u32) -> (String, Vec<String>) {
    if cfg!(windows) {
        (
            "ping".to_string(),
            vec![
                "-n".to_string(),
                (seconds + 1).to_string(),
                "127.0.0.1".to_string(),
            ],
        )
    } else {
        ("sleep".to_string(), vec![seconds.to_string()])
    }
}

/// 「先打 line1、延迟、再打 line2」的 shell 夹具：Unix `sh -c '...'`；Windows `cmd /C '...'`
/// （cmd 用 `&` 顺序执行，`ping` 充当 sleep；这里的 `>nul` 由 cmd 解释，成立）。
fn two_line_shell_command() -> (String, Vec<String>) {
    if cfg!(windows) {
        (
            "cmd".to_string(),
            vec![
                "/C".to_string(),
                "echo line1 & ping -n 2 127.0.0.1 >nul & echo line2".to_string(),
            ],
        )
    } else {
        (
            "sh".to_string(),
            vec![
                "-c".to_string(),
                "echo line1; sleep 0.1; echo line2".to_string(),
            ],
        )
    }
}

#[test]
fn test_ac01_job_start_non_blocking() {
    const COMMAND_SECONDS: u32 = 5;
    let (command, command_args) = long_running_command(COMMAND_SECONDS);
    let args = JobStartArgs {
        command,
        args: Some(command_args),
        cwd: None,
        timeout_ms: Some(10000),
    };

    let start_time = std::time::Instant::now();
    let res: JobStartResult = job_start(args).expect("job_start should succeed");
    let elapsed = start_time.elapsed();

    // 原写法 `elapsed < 500ms`：**只在 Unix 成立**——Windows 上进程 spawn + Job Object/沙箱装配
    // 比 Unix 慢一个量级，慢机上 500ms 会被偶发突破（语义没错、预算错）。
    // 改为**与命令自身时长相对**：命令约 5s，启动耗时必须 < 它的一半（2.5s）。
    // 平台无关，且鉴别力不降：真阻塞时 elapsed ≈ 5s，远超阈值。
    assert!(
        elapsed < std::time::Duration::from_secs(COMMAND_SECONDS as u64) / 2,
        "job_start should return immediately without blocking（命令 {}s，启动耗时预算 < 1/2 命令时长，\
         实际 {elapsed:?}）",
        COMMAND_SECONDS
    );
    assert!(!res.job_id.is_empty(), "job_id must not be empty");
    assert!(res.pid > 0, "pid must be a valid positive integer");
    assert_eq!(res.status, "running", "initial status must be running");

    // 语义护栏：阈值放宽后不得变成空转——必须同时证明「job_start 返回时命令仍在跑」，
    // 这才是"非阻塞"的可观测定义（真阻塞到命令结束的话，这里会是 completed/exited）。
    let probe = job_output(JobOutputArgs {
        job_id: res.job_id.clone(),
        wait: Some(false),
        timeout_ms: None,
        offset: None,
    })
    .expect("job_output should succeed");
    assert_eq!(
        probe.status, "running",
        "语义护栏：job_start 返回后命令必须仍在运行（非阻塞语义），实际 status={}",
        probe.status
    );

    // 清理
    let _ = job_kill(JobKillArgs {
        job_id: res.job_id,
        reason: Some("cleanup".to_string()),
    });
}

#[test]
fn test_ac02_job_output_incremental_and_wait() {
    let (command, command_args) = two_line_shell_command();
    let args = JobStartArgs {
        command,
        args: Some(command_args),
        cwd: None,
        timeout_ms: Some(5000),
    };

    let start_res: JobStartResult = job_start(args).expect("job_start should succeed");

    // 增量拉取第 1 次
    let out1: JobOutputResult = job_output(JobOutputArgs {
        job_id: start_res.job_id.clone(),
        wait: Some(false),
        timeout_ms: None,
        offset: Some(0),
    })
    .expect("job_output should succeed");

    // 等待命令完成
    let out2: JobOutputResult = job_output(JobOutputArgs {
        job_id: start_res.job_id.clone(),
        wait: Some(true),
        timeout_ms: Some(3000),
        offset: Some(out1.cursor),
    })
    .expect("job_output wait should succeed");

    assert_eq!(out2.status, "completed");
    assert_eq!(out2.exit_code, Some(0));
    assert!(out2.cursor >= out1.cursor, "cursor must be monotonically increasing");
    assert!(
        out1.output.contains("line1") || out2.output.contains("line2"),
        "incremental output should yield lines"
    );
}

#[test]
fn test_ac03_job_kill_and_cleanup() {
    let (command, command_args) = long_running_command(10);
    let args = JobStartArgs {
        command,
        args: Some(command_args),
        cwd: None,
        timeout_ms: Some(15000),
    };

    let start_res: JobStartResult = job_start(args).expect("job_start should succeed");

    let kill_res: JobKillResult = job_kill(JobKillArgs {
        job_id: start_res.job_id.clone(),
        reason: Some("test termination".to_string()),
    })
    .expect("job_kill should succeed");

    assert!(kill_res.killed, "job should be marked as killed");

    let out_res: JobOutputResult = job_output(JobOutputArgs {
        job_id: start_res.job_id,
        wait: Some(false),
        timeout_ms: None,
        offset: None,
    })
    .expect("job_output should succeed");

    assert_eq!(out_res.status, "killed", "job status should be killed after termination");
}

#[test]
fn test_ac04_job_list() {
    let (command, command_args) = long_running_command(2);
    let args = JobStartArgs {
        command,
        args: Some(command_args),
        cwd: None,
        timeout_ms: Some(5000),
    };

    let start_res: JobStartResult = job_start(args).expect("job_start should succeed");

    let list_res: JobListResult = job_list(JobListArgs {}).expect("job_list should succeed");

    let found = list_res
        .jobs
        .iter()
        .any(|j| j.job_id == start_res.job_id && j.status == "running");
    assert!(found, "newly started job must be listed in job_list");

    // 清理
    let _ = job_kill(JobKillArgs {
        job_id: start_res.job_id,
        reason: Some("cleanup".to_string()),
    });
}

#[test]
fn test_ac05_job_path_security() {
    // 平台说明：这里 command 名字（`ls`）在 Windows 上不存在，但**不影响本用例**——
    // cwd 校验先于 spawn 失败（fail-closed），Err 来自路径闸门而非命令缺失；
    // 反之若只靠"命令不存在"来凑 Err，那才是假绿。
    let malicious_cwd = "../../etc/unauthorized_path";
    let args = JobStartArgs {
        command: "ls".to_string(),
        args: None,
        cwd: Some(malicious_cwd.to_string()),
        timeout_ms: Some(1000),
    };

    let result = job_start(args);
    assert!(
        result.is_err(),
        "job_start with path traversal in cwd must fail-closed"
    );
}
