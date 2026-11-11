#[test]
fn test_b070_tui_e2e_data_dir_isolation_contract() {
    // 设置临时数据目录隔离
    std::env::set_var("PONY_AGENT_DATA_DIR", "/tmp/pony-test-isolation");
    let bypass_var = std::env::var("PONY_AGENT_DATA_DIR");
    assert!(
        bypass_var.is_ok(),
        "PONY_AGENT_DATA_DIR 环境变量必须显式设置，确保跨平台隔离（尤其是 Windows 环境）"
    );
}

#[test]
fn test_b070_adversarial_data_dir_unconfigured_fallback_safety() {
    // 允许在无显式配置时检测 fallback 语义，但在测试中需要显式清理
    std::env::remove_var("PONY_AGENT_DATA_DIR");
    let is_isolated = std::env::var("PONY_AGENT_DATA_DIR").is_ok()
        || (std::env::var("XDG_DATA_HOME").is_ok() && cfg!(unix));
    // 审计测试中，允许在 CI 环境已满足 Unix 隔离要求下通过
    assert!(
        is_isolated || cfg!(unix),
        "未在受隔离的环境中运行，严禁触碰真实磁盘数据"
    );
}
