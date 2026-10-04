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

#[test]
fn test_ac01_job_start_non_blocking() {
    let args = JobStartArgs {
        command: "sleep".to_string(),
        args: Some(vec!["5".to_string()]),
        cwd: None,
        timeout_ms: Some(10000),
    };

    let start_time = std::time::Instant::now();
    let res: JobStartResult = job_start(args).expect("job_start should succeed");
    let elapsed = start_time.elapsed();

    // 必须是非阻塞启动，返回耗时远小于 sleep 时间
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "job_start should return immediately without blocking"
    );
    assert!(!res.job_id.is_empty(), "job_id must not be empty");
    assert!(res.pid > 0, "pid must be a valid positive integer");
    assert_eq!(res.status, "running", "initial status must be running");

    // 清理
    let _ = job_kill(JobKillArgs {
        job_id: res.job_id,
        reason: Some("cleanup".to_string()),
    });
}

#[test]
fn test_ac02_job_output_incremental_and_wait() {
    let args = JobStartArgs {
        command: "sh".to_string(),
        args: Some(vec![
            "-c".to_string(),
            "echo line1; sleep 0.1; echo line2".to_string(),
        ]),
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
    let args = JobStartArgs {
        command: "sleep".to_string(),
        args: Some(vec!["10".to_string()]),
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
    let args = JobStartArgs {
        command: "sleep".to_string(),
        args: Some(vec!["2".to_string()]),
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
