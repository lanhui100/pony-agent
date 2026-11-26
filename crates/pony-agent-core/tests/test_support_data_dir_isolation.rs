//! 防回归：集成测试（`test-support`）下即使未显式设置 `PONY_AGENT_DATA_DIR`，
//! `default_sqlite_path()` 与 `default_frontend_diagnostics_sqlite_path()` 也绝不能回落到
//! `XDG_DATA_HOME` / 用户真实生产数据目录；而显式设置 `PONY_AGENT_DATA_DIR` 时仍立即生效。

use pony_agent_core::agent::control_plane::HostControlPlaneBuilder;
use pony_agent_core::agent::frontend_diagnostics::default_frontend_diagnostics_sqlite_path;
use pony_agent_core::agent::sqlite_session::default_sqlite_path;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
            Some(val) => std::env::set_var(self.key, val),
            None => std::env::remove_var(self.key),
        }
    }
}

fn unique_temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
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
    let _guard_override = EnvRestore::capture("PONY_AGENT_DATA_DIR");
    let _guard_xdg = EnvRestore::capture("XDG_DATA_HOME");

    let decoy = unique_temp_dir("decoy");
    let explicit = unique_temp_dir("explicit");

    // 1. 未设 PONY_AGENT_DATA_DIR、只设 decoy XDG_DATA_HOME：必须落在进程级 test-support 沙箱。
    std::env::remove_var("PONY_AGENT_DATA_DIR");
    std::env::set_var("XDG_DATA_HOME", &decoy);

    let sqlite_1 = default_sqlite_path();
    let sqlite_2 = default_sqlite_path();
    let diag = default_frontend_diagnostics_sqlite_path();

    assert_eq!(
        sqlite_1, sqlite_2,
        "同一进程内未设 PONY_AGENT_DATA_DIR 时，default_sqlite_path() 必须返回稳定沙箱路径"
    );
    let sqlite_str = sqlite_1.to_string_lossy();
    let diag_str = diag.to_string_lossy();
    assert!(
        sqlite_str.contains("pony-agent-test-support-"),
        "default_sqlite_path() 必须落在 test-support 临时沙箱，实际 = {sqlite_str}"
    );
    assert!(
        diag_str.contains("pony-agent-test-support-"),
        "default_frontend_diagnostics_sqlite_path() 必须落在 test-support 临时沙箱，实际 = {diag_str}"
    );
    assert!(
        !sqlite_1.starts_with(&decoy) && !diag.starts_with(&decoy),
        "未设 PONY_AGENT_DATA_DIR 时绝不能回落到 XDG_DATA_HOME / 生产路径"
    );

    let started = Instant::now();
    let _plane = HostControlPlaneBuilder::new().build();
    assert!(
        started.elapsed().as_secs_f64() < 1.0,
        "无显式注入构造 HostControlPlane 应在 1s 内完成（不得加载生产大库）"
    );
    assert!(
        !decoy.join("PonyAgent").join("sessions.db").exists(),
        "decoy 目录下不得生成 sessions.db"
    );
    assert!(
        !decoy.join("PonyAgent").join("frontend-diagnostics.db").exists(),
        "decoy 目录下不得生成 frontend-diagnostics.db"
    );

    // 2. 显式设置 PONY_AGENT_DATA_DIR：必须立即切换为显式沙箱路径。
    std::env::set_var("PONY_AGENT_DATA_DIR", &explicit);
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
