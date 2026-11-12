//! 长期运行的 IO 卫生单测：TUI 运行日志轮转。
//!
//! 实测事故：单个 `tui-runtime.log` 涨到 552MB 且无上限。轮转承诺：
//! - 未超 64MB 时不动文件（避免每次启动都重命名）；
//! - 超限时移为 `.1`，原路径腾空给后续 append 重建；
//! - 重复轮转覆盖旧归档，不产生 `.2` / `.1.1` 级联。

use super::*;

fn log_path(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pony-tui-log-rotate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join(format!("tui-runtime-{tag}.log"))
}

fn backup_of(log: &std::path::Path) -> std::path::PathBuf {
    let mut backup = log.as_os_str().to_os_string();
    backup.push(".1");
    std::path::PathBuf::from(backup)
}

#[test]
fn rotation_keeps_small_log_in_place() {
    let log = log_path("small");
    std::fs::write(&log, b"small log").expect("seed");
    rotate_runtime_log_if_oversized(&log);
    assert!(log.exists(), "未超限时原文件必须保留");
    assert!(!backup_of(&log).exists(), "未超限时不得产生归档");
    let _ = std::fs::remove_file(&log);
}

#[test]
fn rotation_moves_oversized_log_to_single_backup() {
    let log = log_path("big");
    let file = std::fs::File::create(&log).expect("create");
    file.set_len(TUI_RUNTIME_LOG_MAX_BYTES + 1).expect("set len");
    drop(file);

    rotate_runtime_log_if_oversized(&log);
    assert!(!log.exists(), "超限后原路径必须腾空给 append 重建");
    let backup = backup_of(&log);
    assert_eq!(
        std::fs::metadata(&backup).map(|m| m.len()).ok(),
        Some(TUI_RUNTIME_LOG_MAX_BYTES + 1),
        "归档必须保留原始字节数"
    );

    let file = std::fs::File::create(&log).expect("recreate");
    file.set_len(TUI_RUNTIME_LOG_MAX_BYTES + 2).expect("set len");
    drop(file);
    rotate_runtime_log_if_oversized(&log);
    assert_eq!(
        std::fs::metadata(&backup).map(|m| m.len()).ok(),
        Some(TUI_RUNTIME_LOG_MAX_BYTES + 2),
        "第二次轮转必须覆盖旧归档"
    );
    let mut cascaded = log.as_os_str().to_os_string();
    cascaded.push(".1.1");
    assert!(!std::path::PathBuf::from(cascaded).exists(), "不得级联出 .1.1");

    let _ = std::fs::remove_file(&backup);
}