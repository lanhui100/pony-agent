use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

#[tokio::test]
async fn test_watchdog_hot_refresh_contract() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().to_path_buf();
    
    // 断言初始启动时 watchdog 初始化
    let result = pony_agent_core::skills::watchdog::init_watchdog(&root);
    assert!(result.is_ok(), "Watchdog should initialize properly");
    
    // 写入新增 skill 文件
    let skill_path = root.join("test-skill");
    std::fs::create_dir_all(&skill_path).unwrap();
    std::fs::write(skill_path.join("SKILL.md"), "# Test Skill").unwrap();
    
    // 等待超过去抖窗口 (~200ms)
    tokio::time::sleep(Duration::from_millis(300)).await;
    
    // 断言变更产生且只产生一次 delta_notice
    let notices = pony_agent_core::skills::watchdog::poll_delta_notices();
    assert_eq!(notices.len(), 1, "Should emit exactly one delta_notice after debounce");
}

#[tokio::test]
async fn test_watchdog_hot_refresh_adversarial_rapid_writes() {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path().to_path_buf();
    let _ = pony_agent_core::skills::watchdog::init_watchdog(&root);
    
    let skill_path = root.join("adversarial-skill");
    std::fs::create_dir_all(&skill_path).unwrap();
    
    // 连续高频写入，不满足 200ms 稳定窗口
    for i in 0..5 {
        std::fs::write(skill_path.join(format!("file_{}.txt", i)), "data").unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    // 写入过程中，不应产生过早的通知
    let mid_notices = pony_agent_core::skills::watchdog::poll_delta_notices();
    assert_eq!(mid_notices.len(), 0, "Rapid writes must not trigger premature notices");
    
    // 等待稳定窗口生效
    tokio::time::sleep(Duration::from_millis(300)).await;
    let final_notices = pony_agent_core::skills::watchdog::poll_delta_notices();
    assert_eq!(final_notices.len(), 1, "Debounced burst should emit only one notice");
}
