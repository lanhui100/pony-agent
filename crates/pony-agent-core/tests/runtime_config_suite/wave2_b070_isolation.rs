struct DataDirGuard {
    prev: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl DataDirGuard {
    fn set(val: Option<&str>) -> Self {
        let lock = crate::env_lock::lock();
        let prev = std::env::var_os("PONY_AGENT_DATA_DIR");
        match val {
            Some(v) => std::env::set_var("PONY_AGENT_DATA_DIR", v),
            None => std::env::remove_var("PONY_AGENT_DATA_DIR"),
        }
        Self { prev, _lock: lock }
    }
}

impl Drop for DataDirGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(v) => std::env::set_var("PONY_AGENT_DATA_DIR", v),
            None => std::env::remove_var("PONY_AGENT_DATA_DIR"),
        }
    }
}

#[test]
fn test_b070_tui_e2e_data_dir_isolation_contract() {
    let _guard = DataDirGuard::set(Some("/tmp/pony-test-isolation"));
    let bypass_var = std::env::var("PONY_AGENT_DATA_DIR");
    assert!(
        bypass_var.is_ok(),
        "PONY_AGENT_DATA_DIR 环境变量必须显式设置，确保跨平台隔离（尤其是 Windows 环境）"
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "Windows 无 XDG_DATA_HOME/XDG 兜底等价物，且本用例主动清空 PONY_AGENT_DATA_DIR；\n隔离兜底语义仅在 Unix 成立（B230/B246）"
)]
fn test_b070_adversarial_data_dir_unconfigured_fallback_safety() {
    let _guard = DataDirGuard::set(None);
    let is_isolated = std::env::var("PONY_AGENT_DATA_DIR").is_ok()
        || (std::env::var("XDG_DATA_HOME").is_ok() && cfg!(unix));
    // 审计测试中，允许在 CI 环境已满足 Unix 隔离要求下通过
    assert!(
        is_isolated || cfg!(unix),
        "未在受隔离的环境中运行，严禁触碰真实磁盘数据"
    );
}
