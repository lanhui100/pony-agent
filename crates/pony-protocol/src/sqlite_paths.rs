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
pub fn data_dir_override() -> Option<PathBuf> {
    std::env::var_os("PONY_AGENT_DATA_DIR")
        .map(PathBuf::from)
        .filter(|override_dir| !override_dir.as_os_str().is_empty())
}

/// `test-support` 构建（集成 / `[[bin]]` 测试）的兜底数据目录根：进程内一次求值后固定。
///
/// 未设置 `PONY_AGENT_DATA_DIR` 时，测试进程一律落进系统临时目录下的
/// `pony-agent-test-support-<pid>-<nanos>` 沙箱，切断「测试误读用户真实生产库」的回退。
/// **进程级固定**（而非每次随机）才能让同一进程内的会话库 / 诊断库 / OTel spool
/// 落在同一个沙箱根下；显式 `PONY_AGENT_DATA_DIR` 仍可逐次覆盖。
#[cfg(all(not(test), feature = "test-support"))]
pub fn test_support_fallback_data_dir() -> PathBuf {
    static SANDBOX_ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    SANDBOX_ROOT
        .get_or_init(|| {
            std::env::temp_dir().join(format!(
                "pony-agent-test-support-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or_default()
            ))
        })
        .clone()
}

/// 数据目录根的**单点判决**（会话库 / 诊断库 / 会话 JSON 三处默认路径共用，不各立一套规则）。
///
/// 顺序：显式 `PONY_AGENT_DATA_DIR` → （仅 `test-support` 构建）进程级临时沙箱 → `None`。
/// 生产构建里第二跳恒 `None`，判定链与本函数引入前逐字等价。
pub fn data_dir_root() -> Option<PathBuf> {
    data_dir_override().or_else(test_support_sandbox_root)
}

/// `test-support` 构建（集成 / `[[bin]]` 测试）的沙箱根；其余构建恒 `None`。
fn test_support_sandbox_root() -> Option<PathBuf> {
    #[cfg(all(not(test), feature = "test-support"))]
    {
        Some(test_support_fallback_data_dir())
    }

    #[cfg(any(test, not(feature = "test-support")))]
    {
        None
    }
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
        data_dir_root()
            .or_else(dirs::data_local_dir)
            .or_else(dirs::home_dir)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("PonyAgent")
            .join("sessions.db")
    }
}

/// 单元测试专用的会话数据库路径：按进程 + 时间戳唯一化，位于系统临时目录。
pub fn unique_test_sqlite_path() -> PathBuf {
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
