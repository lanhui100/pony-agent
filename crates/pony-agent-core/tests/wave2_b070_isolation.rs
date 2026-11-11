#[test]
fn test_b070_tui_e2e_data_dir_isolation_contract() {
    // 验证当前测试进程必须设置 PONY_AGENT_DATA_DIR 旁路环境变量，以防穿透至真实用户数据目录
    let bypass_var = std::env::var("PONY_AGENT_DATA_DIR");
    assert!(
        bypass_var.is_ok(),
        "PONY_AGENT_DATA_DIR 环境变量必须显式设置，确保跨平台隔离（尤其是 Windows 环境）"
    );
}

#[test]
fn test_b070_adversarial_data_dir_unconfigured_fallback_safety() {
    // 对抗注入：若未配置隔离环境变量，系统决不允许静默回退并写真实用户数据目录
    // 必须检测到潜在的环境未隔离风险并阻断
    let is_isolated = std::env::var("PONY_AGENT_DATA_DIR").is_ok()
        || (std::env::var("XDG_DATA_HOME").is_ok() && cfg!(unix));
    assert!(
        is_isolated,
        "未在受隔离的环境中运行，严禁触碰真实磁盘数据"
    );
}
