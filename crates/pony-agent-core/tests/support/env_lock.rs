use std::sync::{Mutex, MutexGuard};

static PROCESS_ENV_LOCK: Mutex<()> = Mutex::new(());

/// 同一领域集成测试套件二进制内所有修改或读取共享进程环境 / 全局 OTel 状态的用例共用一把锁。
pub fn lock() -> MutexGuard<'static, ()> {
    PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
