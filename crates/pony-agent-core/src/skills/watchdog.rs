// 实现 Watchdog 逻辑，添加去抖动
pub struct SkillWatchdog {
    pub last_event: Option<std::time::Instant>,
    pub debounce_duration: std::time::Duration,
}

impl SkillWatchdog {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            last_event: None,
            debounce_duration: std::time::Duration::from_millis(debounce_ms),
        }
    }
    
    pub fn should_notify(&mut self) -> bool {
        let now = std::time::Instant::now();
        if let Some(last) = self.last_event {
            if now.duration_since(last) < self.debounce_duration {
                return false;
            }
        }
        self.last_event = Some(now);
        true
    }
}
