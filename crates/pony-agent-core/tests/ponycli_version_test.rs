//! B261：`ponycli --version` / `-V` 的进程级契约。
//!
//! 判据口径：这里断言的是**二进制行为**（stdout 内容 + 退出码），而不是内部函数——
//! 只测函数无法证明「没被参数解析吞掉」，而 B261 的症状正是「没有版本输出」。
//! 版本号本身不写死：真源是 workspace 的 `CARGO_PKG_VERSION`（同 crate，天然同源）。

use std::process::Command;

fn run_version_flag(flag: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ponycli"))
        .arg(flag)
        .output()
        .expect("spawn ponycli")
}

#[test]
fn version_flag_prints_bin_and_package_version_with_zero_exit() {
    let output = run_version_flag("--version");
    assert!(
        output.status.success(),
        "--version 必须退出 0，实得 {:?}（stderr={}）",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("ponycli {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn short_version_flag_is_equivalent() {
    let output = run_version_flag("-V");
    assert!(output.status.success(), "-V 必须退出 0");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("ponycli {}", env!("CARGO_PKG_VERSION"))
    );
}
