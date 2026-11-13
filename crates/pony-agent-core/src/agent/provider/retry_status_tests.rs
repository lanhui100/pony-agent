//! 重试进度观察者的回归用例（从 `provider/tests.rs` 外移，避开单文件规模棘轮）。
//!
//! 断言口径：状态行「正在重试（n/N）…」的唯一真值来源是**真实发生**的重试通知，
//! 分子 = 已失败次数、分母 = 本跳尝试总数（含首次），不得虚构进度。

use super::*;

/// 2026-11-14：重试循环必须把「即将重试」的**真实进度**交给宿主观察者 ——
/// 这是状态行「正在重试（n/N）…」的唯一真值来源（分子=已失败次数、分母=尝试总数）。
#[test]
fn retry_loop_reports_real_progress_to_observer() {
    use std::sync::{Arc, Mutex};

    let seen: Arc<Mutex<Vec<(u32, u32)>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&seen);
    let _observer = crate::agent::retry::install_retry_observer(Box::new(move |notice| {
        captured
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((notice.attempt, notice.max_attempts));
    }));

    let sleeper = crate::agent::retry::FakeSleeper::new();
    let attempts = std::cell::Cell::new(0);
    let _: Result<(), String> = retry_provider_scoped_with_sleeper(
        "decision",
        ProviderRetryScope::Decision,
        || {
            attempts.set(attempts.get() + 1);
            Err("provider 返回错误状态：503 Service Unavailable".to_string())
        },
        &sleeper,
        None,
    );

    let seen = seen
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(attempts.get(), 10, "默认 10 次尝试（含首次）");
    assert_eq!(
        seen.len(),
        9,
        "真正发生的重试是 9 次（末次尝试失败后不再通知），实际 = {seen:?}"
    );
    assert_eq!(
        seen.first().copied(),
        Some((1, 10)),
        "首次重试的分母必须是本跳尝试总数（含首次），实际 = {seen:?}"
    );
    assert_eq!(
        seen.last().copied(),
        Some((9, 10)),
        "最后一次重试的分子必须等于 max_attempts - 1，实际 = {seen:?}"
    );
}
