//! B230：跨平台测试路径工具（仅测试 crate 使用）。

use std::path::{Path, PathBuf};

/// Windows `std::fs::canonicalize` 产出 `\\?\` verbatim 前缀；POSIX shell 夹具（Git Bash）
/// 与部分 Win32 API 不接受该形态。测试统一去前缀，让 Rust IO 与 shell 参数共用同一可读路径。
/// Unix 下为恒等变换。
pub fn deverbatim(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    let normalized = if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw.into_owned()
    };
    PathBuf::from(normalized)
}
