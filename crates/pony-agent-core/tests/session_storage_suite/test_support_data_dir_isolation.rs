//! 防回归：`test-support` 构建（集成 / `[[bin]]` 测试）下，即使**未显式设置**
//! `PONY_AGENT_DATA_DIR`，三处默认数据路径也绝不能回落到 `XDG_DATA_HOME` / 用户真实
//! 生产数据目录；显式设置时仍立即生效。
//!
//! 事故面（`backlog:B1795589917533` → `B1795681671813`）：`#[cfg(test)]` 只覆盖 `--lib`
//! 单元测试；集成测试里依赖 crate 以 `cfg(not(test))` 编译，`SessionStore::new()` 会直接
//! 打开宿主真实的 `~/.local/share/PonyAgent/sessions.db`，触发全量日志 fold，实测 GB 级
//! RSS + 11GiB Swap 颠簸拖垮整机。
//!
//! 判定真源：`pony_protocol::sqlite_paths::data_dir_root()`（单点判决）。
//! 本文件只锁两条契约，不锁实现细节：
//!   (a) 未设旁路 ⇒ 落进程级 `pony-agent-test-support-*` 沙箱且同进程内稳定；
//!   (b) 显式设旁路 ⇒ 立即优先，落该目录下。
//!
//! 纪律：环境变量是进程级全局量，故用 `env_lock` 串行，并用 `EnvRestore` 还原现场。

use std::ffi::OsString;
use std::path::PathBuf;

use pony_agent_core::agent::frontend_diagnostics::default_frontend_diagnostics_sqlite_path;
use pony_agent_core::agent::sqlite_session::default_sqlite_path;

const DATA_DIR_ENV: &str = "PONY_AGENT_DATA_DIR";
const XDG_DATA_HOME_ENV: &str = "XDG_DATA_HOME";

struct EnvRestore {
    key: &'static str,
    prev: Option<OsString>,
}

impl EnvRestore {
    fn capture(key: &'static str) -> Self {
        Self {
            key,
            prev: std::env::var_os(key),
        }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        match &self.prev {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn unique_temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!(
        "pony-it-data-iso-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn test_support_isolates_default_paths_without_override_and_respects_explicit_override() {
    let _serial = crate::env_lock::lock();
    let _guard_override = EnvRestore::capture(DATA_DIR_ENV);
    let _guard_xdg = EnvRestore::capture(XDG_DATA_HOME_ENV);

    let decoy = unique_temp_dir("decoy");
    let explicit = unique_temp_dir("explicit");

    // 1. 未设 PONY_AGENT_DATA_DIR、只设诱饵 XDG_DATA_HOME：必须落进程级 test-support 沙箱。
    std::env::remove_var(DATA_DIR_ENV);
    std::env::set_var(XDG_DATA_HOME_ENV, &decoy);

    let sqlite_first = default_sqlite_path();
    let sqlite_second = default_sqlite_path();
    let diagnostics = default_frontend_diagnostics_sqlite_path();

    assert_eq!(
        sqlite_first, sqlite_second,
        "同一进程内未设 PONY_AGENT_DATA_DIR 时，default_sqlite_path() 必须返回稳定沙箱路径"
    );
    let sqlite_display = sqlite_first.to_string_lossy();
    let diagnostics_display = diagnostics.to_string_lossy();
    assert!(
        sqlite_display.contains("pony-agent-test-support-"),
        "default_sqlite_path() 必须落在 test-support 临时沙箱，实际 = {sqlite_display}"
    );
    assert!(
        diagnostics_display.contains("pony-agent-test-support-"),
        "default_frontend_diagnostics_sqlite_path() 必须落在 test-support 临时沙箱，实际 = {diagnostics_display}"
    );
    assert!(
        !sqlite_first.starts_with(&decoy) && !diagnostics.starts_with(&decoy),
        "未设 PONY_AGENT_DATA_DIR 时绝不能回落到 XDG_DATA_HOME / 生产路径"
    );
    assert!(
        sqlite_display.ends_with("PonyAgent/sessions.db")
            || sqlite_display.ends_with("PonyAgent\\sessions.db"),
        "沙箱内仍需保持 <root>/PonyAgent/sessions.db 布局，实际 = {sqlite_display}"
    );

    // 2. 显式设置 PONY_AGENT_DATA_DIR：必须立即切换为显式沙箱路径。
    std::env::set_var(DATA_DIR_ENV, &explicit);
    assert_eq!(
        default_sqlite_path(),
        explicit.join("PonyAgent").join("sessions.db"),
        "显式 PONY_AGENT_DATA_DIR 必须优先于 test-support 默认沙箱"
    );
    assert_eq!(
        default_frontend_diagnostics_sqlite_path(),
        explicit.join("PonyAgent").join("frontend-diagnostics.db"),
        "显式 PONY_AGENT_DATA_DIR 必须同样生效于 frontend-diagnostics.db"
    );

    let _ = std::fs::remove_dir_all(&decoy);
    let _ = std::fs::remove_dir_all(&explicit);
}
