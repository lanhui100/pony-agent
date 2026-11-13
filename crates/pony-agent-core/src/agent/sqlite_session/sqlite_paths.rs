//! SQLite 会话库的路径解析（生产默认路径 + 测试注入旁路 + 单元测试隔离路径）。
//!
//! 自 `agent/sqlite_session.rs` 拆分（file-size 收口，行为逐字等价）：
//! 三处函数的签名、`cfg` 门与判定逻辑一字未改，父模块通过 re-export 保留原有
//! `sqlite_session::default_sqlite_path` / `sqlite_session::data_dir_override` 寻址契约。

use std::path::PathBuf;

/// 数据目录旁路（**测试注入专用**）：`PONY_AGENT_DATA_DIR` 优先于平台默认数据目录。
///
/// 动机（Windows 全量首跑）：集成测试无法用环境变量重定向数据目录——
/// `dirs::data_local_dir()` 在 Windows 走 known-folder API，不读 `LOCALAPPDATA` /
/// `XDG_DATA_HOME`，于是 `tests/tui_persistence_truthfulness.rs` 的沙箱在 Windows 上失效，
/// 用例既红又读写真实用户库。本旁路与 backlog CLI 的 `PONY_BACKLOG_ROOT` 同构：
/// 仅测试注入用；**未设置时行为逐字不变**（不新增默认路径分支）。
///
/// `cfg(not(test))`：单元测试构建（`cfg(test)`）走 `unique_test_sqlite_path()` 隔离，
/// 该旁路在那里既无调用方也无意义，故不编译，避免 dead_code 噪音。
#[cfg(not(test))]
pub(crate) fn data_dir_override() -> Option<PathBuf> {
    std::env::var_os("PONY_AGENT_DATA_DIR")
        .map(PathBuf::from)
        .filter(|override_dir| !override_dir.as_os_str().is_empty())
}

/// Returns the default SQLite database path.
pub fn default_sqlite_path() -> PathBuf {
    #[cfg(test)]
    {
        // 测试必须隔离：绝不读写用户生产数据库（%LOCALAPPDATA%/PonyAgent/sessions.db）。
        unique_test_sqlite_path()
    }

    #[cfg(not(test))]
    {
        data_dir_override()
            .or_else(dirs::data_local_dir)
            .or_else(dirs::home_dir)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("PonyAgent")
            .join("sessions.db")
    }
}

/// 单元测试专用的会话数据库路径：按进程 + 时间戳唯一化，位于系统临时目录。
#[cfg(test)]
pub(super) fn unique_test_sqlite_path() -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "pony-agent-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ))
        .join("sessions.db")
}
