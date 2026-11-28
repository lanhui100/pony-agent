//! SQLite 会话库的路径解析（薄包装：规则真源在 `pony_protocol::sqlite_paths`）。
//!
//! 这里曾经是 `pony-protocol` 同名文件的逐字副本（两份规则各自演化），2026-11-28
//! 收敛为单源：**目录选择规则**（`PONY_AGENT_DATA_DIR` > 平台数据目录 > `$HOME` > cwd，
//! 末尾追加 `PonyAgent/sessions.db`）与**测试隔离规则**（进程 + 时间戳的唯一临时路径）
//! 都只在 `pony_protocol::sqlite_paths` 维护一份；本文件只决定「当前 crate 的构建形态走哪一支」。
//!
//! 为什么不能直接 `pub use pony_protocol::sqlite_paths::default_sqlite_path`：
//! `cfg(test)` 只对**正在编译的 crate** 生效。`pony-storage` 跑单元测试时，依赖
//! `pony-protocol` 是按 `cfg(not(test))` 编译的，直接 re-export 会让 storage 的单元测试
//! 落到真实用户库。因此隔离分支必须留在本 crate。

use std::path::PathBuf;

/// 数据目录旁路（**测试注入专用**）：真源在 `pony_protocol::sqlite_paths`。
#[cfg(not(test))]
pub use pony_protocol::sqlite_paths::data_dir_override;

/// Returns the default SQLite database path（按本 crate 构建形态选隔离/生产分支）。
pub fn default_sqlite_path() -> PathBuf {
    #[cfg(test)]
    {
        // 测试必须隔离：绝不读写用户生产数据库（%LOCALAPPDATA%/PonyAgent/sessions.db）。
        pony_protocol::sqlite_paths::unique_test_sqlite_path()
    }

    #[cfg(not(test))]
    {
        pony_protocol::sqlite_paths::production_sqlite_path()
    }
}
