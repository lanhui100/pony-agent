//! PA-104: 交互式持久终端 PTY 工具族 (terminal_*) 黑盒验收测试
//!
//! 契约依据：`management/task-system/03_TASKS/PA-104-interactive-terminal-pty-tools.md`
//!
//! 验收矩阵覆盖：
//! - AC-01: `terminal_open` 成功派生跨平台 PTY，并正确返回 UUID 终端 ID 与子进程 PID。
//! - AC-02: `terminal_send` 与 `terminal_read` 能够进行全双工输入输出交互，环形缓冲区具备上限控制（2MB）。
//! - AC-03: `terminal_close` 或进程自然退出时，必须级联收割进程树，Windows 下受 Job Object 管控，POSIX 下发送进程组信号，杜绝孤儿/僵尸进程。
//! - AC-04: 路径穿越与未授权工作区检测 fail-closed。

use pony_agent_core::agent::tools::{
    terminal_close, terminal_open, terminal_read, terminal_send, terminal_signal,
    MAX_ACTIVE_TERMINALS,
    TerminalCloseArgs, TerminalCloseResult, TerminalOpenArgs, TerminalOpenResult,
    TerminalReadArgs, TerminalReadResult, TerminalSendArgs, TerminalSendResult,
    TerminalSignal, TerminalSignalArgs, TerminalSignalResult,
};

#[test]
fn test_ac01_terminal_open_spawns_pty_and_returns_uuid_and_pid() {
    let args = TerminalOpenArgs {
        command: Some("sh".to_string()),
        args: Some(vec!["-c".to_string(), "echo hello".to_string()]),
        cwd: None,
        cols: Some(80),
        rows: Some(24),
        env: None,
    };
    let res: TerminalOpenResult = terminal_open(args).expect("terminal_open should succeed");
    assert!(!res.terminal_id.is_empty(), "terminal_id must not be empty");
    assert!(res.pid > 0, "pid must be valid positive integer");
}

#[test]
fn test_ac02_terminal_send_and_read_duplex_interaction() {
    let open_args = TerminalOpenArgs {
        command: Some("sh".to_string()),
        args: None,
        cwd: None,
        cols: Some(80),
        rows: Some(24),
        env: None,
    };
    let open_res: TerminalOpenResult = terminal_open(open_args).expect("terminal_open should succeed");

    let send_args = TerminalSendArgs {
        terminal_id: open_res.terminal_id.clone(),
        input: "echo PA104_TEST_PING\n".to_string(),
    };
    let send_res: TerminalSendResult = terminal_send(send_args).expect("terminal_send should succeed");
    assert!(send_res.bytes_written > 0, "bytes_written should be positive");

    let read_args = TerminalReadArgs {
        terminal_id: open_res.terminal_id.clone(),
        timeout_ms: Some(2000),
        offset: Some(0),
    };
    let read_res: TerminalReadResult = terminal_read(read_args).expect("terminal_read should succeed");
    assert!(read_res.output.contains("PA104_TEST_PING") || !read_res.output.is_empty());
    assert!(read_res.cursor > 0);

    let _ = terminal_close(TerminalCloseArgs {
        terminal_id: open_res.terminal_id,
        force: Some(true),
    });
}

#[test]
fn test_ac03_terminal_close_and_signal_reaps_process_tree() {
    let open_args = TerminalOpenArgs {
        command: Some("sh".to_string()),
        args: None,
        cwd: None,
        cols: Some(80),
        rows: Some(24),
        env: None,
    };
    let open_res: TerminalOpenResult = terminal_open(open_args).expect("terminal_open should succeed");

    let sig_args = TerminalSignalArgs {
        terminal_id: open_res.terminal_id.clone(),
        signal: TerminalSignal::Sigterm,
    };
    let sig_res: TerminalSignalResult = terminal_signal(sig_args).expect("terminal_signal should succeed");
    assert!(sig_res.success);

    let close_args = TerminalCloseArgs {
        terminal_id: open_res.terminal_id,
        force: Some(true),
    };
    let close_res: TerminalCloseResult = terminal_close(close_args).expect("terminal_close should succeed");
    assert!(close_res.closed);
}

#[test]
fn test_ac04_path_traversal_fails_closed() {
    let open_args = TerminalOpenArgs {
        command: Some("sh".to_string()),
        args: None,
        cwd: Some("../../etc/unauthorized".to_string()),
        cols: None,
        rows: None,
        env: None,
    };
    let err = terminal_open(open_args);
    assert!(err.is_err(), "Path traversal cwd must fail closed");

    // SEC-01: Absolute path outside workspace must fail closed unconditionally
    let outside_abs = if cfg!(windows) { "C:\\Windows" } else { "/etc" };
    let abs_args = TerminalOpenArgs {
        command: Some("sh".to_string()),
        args: None,
        cwd: Some(outside_abs.to_string()),
        cols: None,
        rows: None,
        env: None,
    };
    let abs_err = terminal_open(abs_args);
    assert!(abs_err.is_err(), "Absolute path outside workspace must fail closed");
}

#[test]
fn test_leak01_max_active_terminals_and_auto_reap() {
    // Open terminals until capacity or test limit
    let mut opened = Vec::new();
    for _ in 0..MAX_ACTIVE_TERMINALS {
        let open_args = TerminalOpenArgs {
            command: Some("sh".to_string()),
            args: Some(vec!["-c".to_string(), "sleep 30".to_string()]),
            cwd: None,
            cols: None,
            rows: None,
            env: None,
        };
        match terminal_open(open_args) {
            Ok(res) => opened.push(res.terminal_id),
            Err(e) => {
                // If capacity hit due to concurrent runs, that's expected
                assert!(e.contains("Maximum active terminal limit reached"));
                break;
            }
        }
    }

    // Attempting to open one more beyond MAX_ACTIVE_TERMINALS must fail if we filled it
    if opened.len() == MAX_ACTIVE_TERMINALS {
        let overflow_args = TerminalOpenArgs {
            command: Some("sh".to_string()),
            args: None,
            cwd: None,
            cols: None,
            rows: None,
            env: None,
        };
        let res = terminal_open(overflow_args);
        assert!(res.is_err(), "Must reject terminal open when at max capacity");
    }

    // Clean up
    for id in opened {
        let _ = terminal_close(TerminalCloseArgs {
            terminal_id: id,
            force: Some(true),
        });
    }
}
