//! W36 / PA-145 契约：`recovery` 的 L1 瞬态判据必须与 `retry` 的单一分类出口一致。
//!
//! 缺陷：`recovery.rs:160-169` 保留第二套裸 `contains` 码表（timeout/connection/429/502/
//! 503/504/529）且**不做 URL/端口剥离**，与 `retry.rs` 的 `ProviderRetryPolicy::classify`
//! 构成平行判据。wave-35 已把 retry 侧改为「剥离 endpoint 片段后再匹配」，recovery 侧未跟随 ⇒
//! 同一段错误文本在 recovery 判 `RetryWithDelay`、在 retry 判 `NonRetryable`（判据漂移）。
//!
//! 本测试锁「同一错误文本在两个出口的分类结论必须一致」，并同时覆盖 PA-143/PA-144 的
//! 数字污染文本；收敛到 retry 单一出口后转绿。

use pony_agent_core::agent::recovery::{
    ExecutionRecoveryPolicy, RecoveryAction, RecoveryContext, RecoveryLayer, RecoveryPipelineConfig,
};
use pony_agent_core::agent::retry::{BackoffConfig, ProviderRetryPolicy, RetryBudget};

/// 构造一个**只受瞬态判据影响**的 L1 上下文：无 stream_state（跳过流降级分支）、
/// 预算未耗尽、无 fallback 模型（跳过 529 倒换分支）。
fn recovery_action(raw_error: &str) -> RecoveryAction {
    let policy = ExecutionRecoveryPolicy::new(RecoveryPipelineConfig::default());
    let budget = RetryBudget::new(3, 10_000);
    let ctx = RecoveryContext {
        layer: RecoveryLayer::L1TransportUpstream,
        raw_error: raw_error.to_string(),
        error_code: None,
        stream_state: None,
        current_attempt: 0,
        consecutive_failures: 1,
        primary_model: Some("claude-3-5-sonnet".to_string()),
        fallback_model: None,
        tool_name: None,
        tool_is_idempotent: true,
    };
    policy.decide(&ctx, &budget)
}

#[test]
fn l1_recovery_and_retry_classification_agree_on_same_error_text() {
    let retry_policy = ProviderRetryPolicy::new(BackoffConfig::default());
    // (错误文本, 期望是否瞬态可重试)。
    // 瞬态侧锁住两个出口的既有合法命中；非瞬态侧锁住 PA-143/PA-144 的污染文本
    // （端口数字 / 长数字串内片段 / JSON 错误码片段）不得伪造瞬态。
    let cases: &[(&str, bool)] = &[
        ("operation timed out", true),
        ("HTTP 429 Too Many Requests", true),
        ("status:502", true),
        ("connection reset by peer", true),
        ("provider stream aborted; elapsed=40832ms", false),
        (
            "{\"error\":{\"message\":\"invalid request\",\"code\":\"429001\"}}",
            false,
        ),
        (
            "解析 provider SSE 流失败: 未找到 SSE data 事件; endpoint=127.0.0.1:42953/v1",
            false,
        ),
        (
            "解析 provider SSE 流失败: 未找到 SSE data 事件; endpoint=localhost:50208",
            false,
        ),
        (
            "响应预览: http://127.0.0.1:42953/v1; 未找到 SSE data 事件",
            false,
        ),
    ];

    for (text, expected_retryable) in cases {
        let retry_kind = retry_policy.classify(text);
        let retry_retryable = retry_kind.is_retryable();
        let action = recovery_action(text);
        let recovery_retryable = matches!(action, RecoveryAction::RetryWithDelay { .. });

        assert_eq!(
            retry_retryable, *expected_retryable,
            "retry 分类与契约不一致: {text} -> {retry_kind:?}"
        );
        assert_eq!(
            recovery_retryable, *expected_retryable,
            "recovery L1 分类与契约不一致（平行 contains 判据漂移）: {text} -> {action:?}"
        );
        assert_eq!(
            recovery_retryable, retry_retryable,
            "同一错误文本在 recovery 与 retry 两个出口结论必须一致: {text}"
        );
    }
}
