//! 退避窗口封顶、Always 档位与 RetryNotifier 真实延迟的回归测试（从 `retry.rs` 外移，守住单文件规模棘轮）。

use super::*;

#[test]
fn decide_aborts_when_window_exceeds_single_wait_cap_using_raw_value() {
    let policy = ProviderRetryPolicy::new(BackoffConfig {
        initial_delay_ms: 500,
        multiplier: 2.0,
        max_delay_ms: 8_000,
        max_retries: 3,
        total_budget_ms: 20 * 60 * 1_000,
        jitter_kind: JitterKind::None,
    });
    let failure = FailureKind::TransientRetryable {
        details: "503".to_string(),
    };
    let budget = RetryBudget::new(3, 20 * 60 * 1_000);
    let ra = RetryAfter {
        seconds: Some(30 * 60),
    };
    let decision = policy.decide(&failure, &budget, 1, Some(&ra), StreamState::NoDelta);
    match decision {
        RetryDecision::Abort { reason } => {
            assert!(
                reason.contains("1800000"),
                "诊断必须报原始窗口 30min=1800000ms（不得被截成 600000），实际={reason}"
            );
        }
        other => panic!("超单次等待封顶必须 Abort，实际={other:?}"),
    }
}

#[test]
fn always_mode_window_only_raises_never_lowers_curve() {
    let policy = ProviderRetryPolicy::new(BackoffConfig {
        initial_delay_ms: 20_000,
        multiplier: 2.0,
        max_delay_ms: 40_000,
        max_retries: 2,
        total_budget_ms: 90_000,
        jitter_kind: JitterKind::None,
    })
    .with_mode(RetryMode::Always);
    let failure = FailureKind::TransientRetryable {
        details: "503".to_string(),
    };
    let budget = RetryBudget::new(2, 90_000);
    let ra = RetryAfter { seconds: Some(1) };
    let decision = policy.decide(&failure, &budget, 1, Some(&ra), StreamState::NoDelta);
    match decision {
        RetryDecision::Retry { delay_ms } => {
            assert_eq!(
                delay_ms, 20_000,
                "短窗口不得把已抬起的 20s 曲线压回 1s"
            );
        }
        other => panic!("Always 必须 Retry，实际={other:?}"),
    }
}

#[test]
fn retry_notice_reports_actual_delay_not_recomputed_curve() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    let seen = Arc::new(AtomicU64::new(u64::MAX));
    let seen_for_observer = Arc::clone(&seen);
    let _guard = install_retry_observer(Box::new(move |notice: &RetryNotice| {
        seen_for_observer.store(notice.delay_ms, Ordering::SeqCst);
    }));

    let notifier = RetryNotifier::new("followup_sync", "followup");
    let config = BackoffConfig::default();
    notifier.notify(1, &config, "provider 返回错误状态：503；retry_after=90", 90_000);

    assert_eq!(
        seen.load(Ordering::SeqCst),
        90_000,
        "通知里的 delay_ms 必须是真实退避值（90s），不得自算成曲线 500ms"
    );
}
