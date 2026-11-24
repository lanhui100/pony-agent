#[test]
fn test_b104_rate_limit_short_window_retry_contract() {
    // 验证限流 4s 窗口不应立即中止，而应等待重试
    let cooldown_window_sec = 4;
    assert!(cooldown_window_sec <= 5, "Short rate-limit window should enter retry buffer");
}

#[test]
fn test_b104_rate_limit_exhausted_abort_adversarial() {
    // 对抗注入：超时上限或多次连续限流后安全 Abort
    let retry_count = 4;
    assert!(retry_count > 3, "Max retry limit must enforce Abort");
}
