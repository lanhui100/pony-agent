use std::time::Duration;

/// Backoff configuration for retry policies.
#[derive(Debug, Clone, Copy)]
pub struct BackoffConfig {
    pub initial_delay_ms: u64,
    pub multiplier: f64,
    pub max_delay_ms: u64,
    pub max_retries: u32,
    pub total_budget_ms: u64,
    pub jitter_kind: JitterKind,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 400,
            multiplier: 2.0,
            max_delay_ms: 5_000,
            max_retries: 3,
            total_budget_ms: 12_000,
            jitter_kind: JitterKind::Full,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitterKind {
    None,
    Equal,
    Full,
}

/// Pure delay planner: given attempt index (1-based), returns the delay before that attempt.
/// attempt=0 (first call) returns Duration::ZERO.
pub fn compute_delay(attempt: u32, config: &BackoffConfig, jitter_sample: f64) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    let base = config.initial_delay_ms as f64 * config.multiplier.powi(attempt as i32 - 1);
    let clamped = base.min(config.max_delay_ms as f64);
    let delay = match config.jitter_kind {
        JitterKind::None => clamped,
        JitterKind::Equal => clamped * (0.5 + jitter_sample * 0.5),
        JitterKind::Full => jitter_sample * clamped,
    };
    Duration::from_millis(delay as u64)
}

/// Retry decision for the escalation contract.
#[derive(Debug, Clone, PartialEq)]
pub enum RetryDecision {
    Retry {
        delay_ms: u64,
    },
    Fallback {
        target: FallbackTarget,
        reason: String,
    },
    Escalate {
        reason: String,
    },
    Abort {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum FallbackTarget {
    Sync,
    LocalSynthesis,
}

/// Stream output commitment state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    PreConnection,
    NoDelta,
    ReasoningOnly,
    VisibleTextStarted,
    ToolCallStarted,
}

impl StreamState {
    pub fn may_auto_retry(self) -> bool {
        matches!(
            self,
            Self::PreConnection | Self::NoDelta | Self::ReasoningOnly
        )
    }

    pub fn may_stream_to_sync_fallback(self) -> bool {
        matches!(self, Self::NoDelta | Self::ReasoningOnly)
    }

    pub fn transition(self, kind: StreamEventKind) -> Self {
        if self == Self::ToolCallStarted {
            return Self::ToolCallStarted;
        }
        if matches!(kind, StreamEventKind::Text) {
            return Self::VisibleTextStarted;
        }
        if matches!(kind, StreamEventKind::MixedReasoningText) {
            return Self::VisibleTextStarted;
        }
        match kind {
            StreamEventKind::Reasoning if self == Self::NoDelta || self == Self::PreConnection => {
                Self::ReasoningOnly
            }
            StreamEventKind::ToolCall => Self::ToolCallStarted,
            StreamEventKind::ConnectionEstablished if self == Self::PreConnection => Self::NoDelta,
            _ => self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEventKind {
    ConnectionEstablished,
    Reasoning,
    Text,
    MixedReasoningText,
    ToolCall,
}

/// Provider failure classification.
#[derive(Debug, Clone, PartialEq)]
pub enum FailureKind {
    TransientRetryable { details: String },
    NonRetryable { details: String },
    RequiresRequestMutation { details: String },
    UnsafeToRetry { details: String },
}

impl FailureKind {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::TransientRetryable { .. })
    }
}

/// Marker type for retry cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

/// Sleeper trait for injectable sleep in tests.
/// Returns `Err(Cancelled)` if the sleep was interrupted.
pub trait Sleeper: Send + Sync {
    fn sleep(
        &self,
        duration: Duration,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), Cancelled>;
}

pub struct StdThreadSleeper;

impl Sleeper for StdThreadSleeper {
    fn sleep(
        &self,
        duration: Duration,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), Cancelled> {
        let start = std::time::Instant::now();
        while start.elapsed() < duration {
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(Cancelled);
            }
            std::thread::sleep(std::cmp::min(
                std::time::Duration::from_millis(50),
                duration.saturating_sub(start.elapsed()),
            ));
        }
        Ok(())
    }
}

pub struct FakeSleeper {
    pub total_slept: std::sync::atomic::AtomicU64,
}

impl FakeSleeper {
    pub fn new() -> Self {
        Self {
            total_slept: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl Sleeper for FakeSleeper {
    fn sleep(
        &self,
        duration: Duration,
        _cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), Cancelled> {
        self.total_slept.fetch_add(
            duration.as_millis() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        Ok(())
    }
}

/// Budget tracker for request-level retry.
#[derive(Debug, Clone)]
pub struct RetryBudget {
    pub max_retries: u32,
    pub total_budget_ms: u64,
    pub elapsed_ms: u64,
    pub attempts_used: u32,
}

impl RetryBudget {
    pub fn new(max_retries: u32, total_budget_ms: u64) -> Self {
        Self {
            max_retries,
            total_budget_ms,
            elapsed_ms: 0,
            attempts_used: 0,
        }
    }

    pub fn is_exhausted(&self) -> bool {
        self.attempts_used >= self.max_retries || self.elapsed_ms >= self.total_budget_ms
    }

    pub fn exhausted_reason(&self) -> Option<&'static str> {
        if self.is_exhausted() {
            if self.attempts_used >= self.max_retries {
                Some("attempt_limit_exhausted")
            } else {
                Some("time_budget_exhausted")
            }
        } else {
            None
        }
    }

    pub fn record_attempt(&mut self, delay_ms: u64) {
        self.attempts_used += 1;
        self.elapsed_ms = self.elapsed_ms.saturating_add(delay_ms);
    }

    pub fn record_execution(&mut self, execution_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(execution_ms);
    }
}

/// Retry-After header handling.
#[derive(Debug, Clone)]
pub struct RetryAfter {
    pub seconds: Option<u64>,
}

impl RetryAfter {
    pub fn parse(header_value: &str) -> Self {
        if let Ok(secs) = header_value.trim().parse::<u64>() {
            return Self {
                seconds: Some(secs),
            };
        }
        // HTTP-date format parsing is deferred to a follow-up.
        // The seconds format covers the majority of Retry-After usage (429/503).
        Self { seconds: None }
    }

    pub fn effective_delay(&self, backoff_delay_ms: u64, remaining_budget_ms: u64) -> Option<u64> {
        let ra = self.seconds?;
        let ra_ms = ra.saturating_mul(1000);
        if ra_ms == 0 || ra_ms > remaining_budget_ms {
            return None;
        }
        Some(backoff_delay_ms.max(ra_ms).min(remaining_budget_ms))
    }
}

/// Provider-specific retry policy combining config, classification, and budget.
pub struct ProviderRetryPolicy {
    pub config: BackoffConfig,
    pub sleeper: Box<dyn Sleeper>,
}

impl ProviderRetryPolicy {
    pub fn new(config: BackoffConfig) -> Self {
        Self {
            config,
            sleeper: Box::new(StdThreadSleeper),
        }
    }

    pub fn with_sleeper(config: BackoffConfig, sleeper: Box<dyn Sleeper>) -> Self {
        Self { config, sleeper }
    }

    /// Classify a provider error based on its message content.
    /// This does NOT consider stream state; stream safety is handled by `decide`.
    pub fn classify(&self, err: &str) -> FailureKind {
        let lower = err.to_ascii_lowercase();
        if lower.contains("type=timeout")
            || lower.contains("timeout")
            || lower.contains("timed out")
            || lower.contains("deadline has elapsed")
        {
            return FailureKind::TransientRetryable {
                details: err.to_string(),
            };
        }
        if lower.contains("429") || lower.contains("rate limit") || lower.contains("rate_limit") {
            return FailureKind::TransientRetryable {
                details: err.to_string(),
            };
        }
        if lower.contains("502")
            || lower.contains("503")
            || lower.contains("504")
            || lower.contains("408")
        {
            return FailureKind::TransientRetryable {
                details: err.to_string(),
            };
        }
        if lower.contains("connection reset")
            || lower.contains("connection refused")
            || lower.contains("dns")
        {
            return FailureKind::TransientRetryable {
                details: err.to_string(),
            };
        }
        if lower.contains("400")
            || lower.contains("401")
            || lower.contains("403")
            || lower.contains("404")
            || lower.contains("422")
            || lower.contains("407")
            || lower.contains("413")
        {
            return FailureKind::NonRetryable {
                details: err.to_string(),
            };
        }
        if lower.contains("context too large")
            || lower.contains("context_length")
            || lower.contains("max_tokens")
            || lower.contains("payload too large")
        {
            return FailureKind::RequiresRequestMutation {
                details: err.to_string(),
            };
        }
        FailureKind::NonRetryable {
            details: err.to_string(),
        }
    }

    /// Decide what to do with a failure given stream state and budget.
    /// Stream safety is checked here: states with visible output or tool-call commitment
    /// prohibit auto-retry and produce Abort.
    /// Fallback to sync mode should be decided at a higher orchestrator level.
    pub fn decide(
        &self,
        failure: &FailureKind,
        budget: &RetryBudget,
        attempt: u32,
        retry_after: Option<&RetryAfter>,
        stream_state: StreamState,
    ) -> RetryDecision {
        if !stream_state.may_auto_retry() {
            return RetryDecision::Abort {
                reason: format!(
                    "stream state {:?} prohibits auto-retry: {}",
                    stream_state,
                    failure.details()
                ),
            };
        }
        if !failure.is_retryable() {
            return RetryDecision::Abort {
                reason: format!("{:?}: {}", failure, failure.details()),
            };
        }
        if budget.is_exhausted() {
            return RetryDecision::Escalate {
                reason: format!("budget exhausted: {:?}", budget.exhausted_reason()),
            };
        }
        let base_delay = compute_delay(attempt, &self.config, 0.5);
        let delay = if let Some(ra) = retry_after {
            let remaining = self
                .config
                .total_budget_ms
                .saturating_sub(budget.elapsed_ms);
            ra.effective_delay(base_delay.as_millis() as u64, remaining)
                .unwrap_or(base_delay.as_millis() as u64)
        } else {
            base_delay.as_millis() as u64
        };
        RetryDecision::Retry {
            delay_ms: delay.min(self.config.max_delay_ms),
        }
    }

    /// Check if fallback to sync mode is appropriate for this failure+stream combination.
    /// Stream→sync fallback is safe when the stream has not yet committed visible text
    /// (NoDelta or ReasoningOnly) AND the error is non-retryable.
    /// This is a higher-level policy decision, not part of the auto-retry loop.
    pub fn should_fallback_to_sync(
        &self,
        failure: &FailureKind,
        stream_state: StreamState,
    ) -> bool {
        !failure.is_retryable() && stream_state.may_stream_to_sync_fallback()
    }
}

impl FailureKind {
    pub fn details(&self) -> &str {
        match self {
            Self::TransientRetryable { details }
            | Self::NonRetryable { details }
            | Self::RequiresRequestMutation { details }
            | Self::UnsafeToRetry { details } => details,
        }
    }

    pub fn is_unsafe_to_retry(&self) -> bool {
        matches!(self, Self::UnsafeToRetry { .. })
    }

    pub fn requires_request_mutation(&self) -> bool {
        matches!(self, Self::RequiresRequestMutation { .. })
    }
}

/// PA-100：rate-limit 特征判定（单一来源）。`classify` 仍把这类错误统一归为
/// `TransientRetryable`（可重试）；本函数供重试调度方进一步区分"应使用长退避
/// 覆盖分钟级限流窗口"的子类，避免在调用侧各自维护关键词列表造成漂移。
///
/// 消费前提（PA-100 code-review A P2-2）：仅当 `classify` 先判为可重试时本判定
/// 才会被调度方消费——"tpm"/"quota"/"too many requests"/"ratelimit" 等措辞若
/// 单独出现（报文不含 "429"/"rate limit"/timeout 特征），classify 会落默认
/// NonRetryable 分支直接 Abort，长退避不会触发。方向安全：不在永久配额耗尽上烧退避。
///
/// 判定优先于措辞细节：报文同时含 timeout 与 rate-limit 特征时（如网关把上游
/// 429 包装成超时文案），按 rate-limit 处理——长退避对两类瞬时故障都安全，
/// 反向（短退避撞限流窗口）则必然失败。
pub fn is_rate_limit_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("ratelimit")
        || lower.contains("too many requests")
        || lower.contains("tpm")
        || lower.contains("quota")
}

/// Convenience function: run a full retry loop with the given policy.
/// Returns `Ok(result)` on success, or the last `RetryDecision` on exhaustion.
pub fn retry_with_policy<T, F>(
    label: &str,
    policy: &ProviderRetryPolicy,
    mut operation: F,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<T, RetryDecision>
where
    F: FnMut() -> Result<T, (String, StreamState)>,
{
    let mut budget = RetryBudget::new(policy.config.max_retries, policy.config.total_budget_ms);
    let mut last_decision = RetryDecision::Abort {
        reason: "no attempts made".to_string(),
    };
    let start = std::time::Instant::now();

    for attempt in 0..=policy.config.max_retries {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(RetryDecision::Abort {
                reason: "cancelled".to_string(),
            });
        }

        let delay = compute_delay(attempt, &policy.config, 0.5);
        if attempt > 0 {
            if let Err(d) = policy.sleeper.sleep(delay, cancelled) {
                return Err(RetryDecision::Abort {
                    reason: format!("sleep cancelled: {:?}", d),
                });
            }
            budget.record_attempt(delay.as_millis() as u64);
        }

        match operation() {
            Ok(value) => {
                if attempt > 0 {
                    eprintln!(
                        "{}: succeeded on attempt {}/{}",
                        label, attempt, policy.config.max_retries
                    );
                }
                budget.record_execution(start.elapsed().as_millis() as u64);
                return Ok(value);
            }
            Err((err, stream_state)) => {
                let failure = policy.classify(&err);
                if attempt > 0 {
                    eprintln!(
                        "{}: attempt {}/{} failed: {} (class={:?})",
                        label, attempt, policy.config.max_retries, err, failure
                    );
                }
                let decision = policy.decide(&failure, &budget, attempt, None, stream_state);
                match &decision {
                    RetryDecision::Retry { .. } => {
                        last_decision = decision;
                        // continue loop
                    }
                    RetryDecision::Fallback { .. }
                    | RetryDecision::Escalate { .. }
                    | RetryDecision::Abort { .. } => {
                        return Err(decision);
                    }
                }
            }
        }
    }
    Err(last_decision)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_delay_first_attempt_is_zero() {
        let cfg = BackoffConfig::default();
        assert_eq!(compute_delay(0, &cfg, 0.0), Duration::ZERO);
    }

    #[test]
    fn compute_delay_exponential() {
        let cfg = BackoffConfig {
            initial_delay_ms: 500,
            multiplier: 2.0,
            max_delay_ms: 8000,
            jitter_kind: JitterKind::None,
            ..Default::default()
        };
        assert_eq!(compute_delay(1, &cfg, 0.0).as_millis(), 500);
        assert_eq!(compute_delay(2, &cfg, 0.0).as_millis(), 1000);
        assert_eq!(compute_delay(3, &cfg, 0.0).as_millis(), 2000);
        assert_eq!(compute_delay(4, &cfg, 0.0).as_millis(), 4000);
        assert_eq!(compute_delay(5, &cfg, 0.0).as_millis(), 8000);
    }

    #[test]
    fn compute_delay_respects_max() {
        let cfg = BackoffConfig {
            initial_delay_ms: 1000,
            multiplier: 4.0,
            max_delay_ms: 3000,
            jitter_kind: JitterKind::None,
            ..Default::default()
        };
        assert_eq!(compute_delay(2, &cfg, 0.0).as_millis(), 3000);
    }

    #[test]
    fn stream_state_transitions() {
        let s = StreamState::PreConnection;
        assert_eq!(
            s.transition(StreamEventKind::ConnectionEstablished),
            StreamState::NoDelta
        );
        let s = StreamState::NoDelta;
        assert_eq!(
            s.transition(StreamEventKind::Reasoning),
            StreamState::ReasoningOnly
        );
        let s = StreamState::ReasoningOnly;
        assert_eq!(
            s.transition(StreamEventKind::Text),
            StreamState::VisibleTextStarted
        );
        let s = StreamState::ReasoningOnly;
        assert_eq!(
            s.transition(StreamEventKind::MixedReasoningText),
            StreamState::VisibleTextStarted
        );
        let s = StreamState::NoDelta;
        assert_eq!(
            s.transition(StreamEventKind::ToolCall),
            StreamState::ToolCallStarted
        );
    }

    #[test]
    fn stream_state_may_auto_retry() {
        assert!(StreamState::PreConnection.may_auto_retry());
        assert!(StreamState::NoDelta.may_auto_retry());
        assert!(StreamState::ReasoningOnly.may_auto_retry());
        assert!(!StreamState::VisibleTextStarted.may_auto_retry());
        assert!(!StreamState::ToolCallStarted.may_auto_retry());
    }

    #[test]
    fn retry_budget_exhausted_attempt_limit() {
        let mut budget = RetryBudget::new(2, 10000);
        assert!(!budget.is_exhausted());
        budget.record_attempt(100);
        assert!(!budget.is_exhausted());
        budget.record_attempt(100);
        assert!(budget.is_exhausted());
        assert_eq!(budget.exhausted_reason(), Some("attempt_limit_exhausted"));
    }

    #[test]
    fn retry_budget_exhausted_time() {
        let mut budget = RetryBudget::new(10, 1000);
        assert!(!budget.is_exhausted());
        budget.record_attempt(1100);
        assert!(budget.is_exhausted());
        assert_eq!(budget.exhausted_reason(), Some("time_budget_exhausted"));
    }

    #[test]
    fn failure_classification_timeout_is_retryable() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let kind = policy.classify("operation timed out; type=timeout");
        assert!(kind.is_retryable());
    }

    #[test]
    fn failure_classification_400_is_non_retryable() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let kind = policy.classify("HTTP 400 Bad Request");
        assert!(!kind.is_retryable());
        assert!(matches!(kind, FailureKind::NonRetryable { .. }));
    }

    #[test]
    fn failure_classification_context_too_large_is_requires_mutation() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let kind = policy.classify("context too large: 128k tokens");
        assert!(!kind.is_retryable());
        assert!(matches!(kind, FailureKind::RequiresRequestMutation { .. }));
    }

    #[test]
    fn failure_classification_timeout_at_visible_text_is_unsafe_via_decide() {
        // classify no longer checks stream state; decide does.
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let kind = policy.classify("timeout");
        assert!(kind.is_retryable());
        let budget = RetryBudget::new(3, 10000);
        // decide sees VisibleTextStarted and aborts even for retryable errors
        let decision = policy.decide(&kind, &budget, 1, None, StreamState::VisibleTextStarted);
        assert!(matches!(decision, RetryDecision::Abort { .. }));
    }

    #[test]
    fn retry_after_parse_seconds() {
        let ra = RetryAfter::parse("30");
        assert_eq!(ra.seconds, Some(30));
    }

    #[test]
    fn retry_after_parse_zero() {
        let ra = RetryAfter::parse("0");
        assert!(ra.seconds.is_some());
        let delay = ra.effective_delay(500, 10000);
        assert_eq!(delay, None);
    }

    #[test]
    fn retry_after_exceeds_budget() {
        let ra = RetryAfter { seconds: Some(60) };
        let delay = ra.effective_delay(500, 10000);
        assert_eq!(delay, None);
    }

    #[test]
    fn retry_after_effective_delay() {
        let ra = RetryAfter { seconds: Some(3) };
        let delay = ra.effective_delay(500, 10000);
        assert_eq!(delay, Some(3000));
    }

    #[test]
    fn decide_aborts_on_non_retryable() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::NonRetryable {
            details: "400".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        let decision = policy.decide(&failure, &budget, 1, None, StreamState::NoDelta);
        assert!(matches!(decision, RetryDecision::Abort { .. }));
    }

    #[test]
    fn decide_escalates_on_exhausted_budget() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::TransientRetryable {
            details: "timeout".to_string(),
        };
        let mut budget = RetryBudget::new(1, 10000);
        budget.record_attempt(500);
        let decision = policy.decide(&failure, &budget, 2, None, StreamState::NoDelta);
        assert!(matches!(decision, RetryDecision::Escalate { .. }));
    }

    #[test]
    fn decide_aborts_on_unsafe_with_any_state() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::UnsafeToRetry {
            details: "stream error".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        // UnsafeToRetry + any state → Abort (may_auto_retry doesn't matter for UnsafeToRetry)
        let decision = policy.decide(&failure, &budget, 1, None, StreamState::NoDelta);
        assert!(matches!(decision, RetryDecision::Abort { .. }));
    }

    #[test]
    fn decide_aborts_on_visible_text_state() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::TransientRetryable {
            details: "timeout".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        // VisibleTextStarted prohibits auto-retry even for transient errors
        let decision = policy.decide(&failure, &budget, 1, None, StreamState::VisibleTextStarted);
        assert!(matches!(decision, RetryDecision::Abort { .. }));
    }

    #[test]
    fn should_fallback_to_sync_returns_true_for_non_retryable_in_reasoning() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::NonRetryable {
            details: "server error".to_string(),
        };
        assert!(policy.should_fallback_to_sync(&failure, StreamState::ReasoningOnly));
        assert!(policy.should_fallback_to_sync(&failure, StreamState::NoDelta));
    }

    #[test]
    fn should_fallback_to_sync_returns_false_for_visible_text() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::NonRetryable {
            details: "server error".to_string(),
        };
        assert!(!policy.should_fallback_to_sync(&failure, StreamState::VisibleTextStarted));
        assert!(!policy.should_fallback_to_sync(&failure, StreamState::ToolCallStarted));
    }

    #[test]
    fn decide_retries_on_retryable() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::TransientRetryable {
            details: "timeout".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        let decision = policy.decide(&failure, &budget, 1, None, StreamState::NoDelta);
        assert!(matches!(decision, RetryDecision::Retry { .. }));
    }

    #[test]
    fn decide_aborts_on_requires_mutation() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::RequiresRequestMutation {
            details: "context too large".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        let decision = policy.decide(&failure, &budget, 1, None, StreamState::NoDelta);
        assert!(matches!(decision, RetryDecision::Abort { .. }));
    }

    #[test]
    fn failure_kind_details_is_pub() {
        let f = FailureKind::TransientRetryable {
            details: "test-detail".to_string(),
        };
        assert_eq!(f.details(), "test-detail");
    }

    #[test]
    fn fake_sleeper_records_total() {
        use std::sync::atomic::AtomicBool;
        let sleeper = FakeSleeper::new();
        let cancelled = AtomicBool::new(false);
        assert!(sleeper
            .sleep(Duration::from_millis(150), &cancelled)
            .is_ok());
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            150
        );
    }

    #[test]
    fn retry_with_policy_succeeds_on_first_try() {
        use std::sync::atomic::AtomicBool;
        let policy = ProviderRetryPolicy::with_sleeper(
            BackoffConfig::default(),
            Box::new(FakeSleeper::new()),
        );
        let cancelled = AtomicBool::new(false);
        let result = retry_with_policy(
            "test",
            &policy,
            || Ok::<_, (String, StreamState)>(42),
            &cancelled,
        );
        assert_eq!(result, Ok(42));
    }

    #[test]
    fn retry_with_policy_aborts_on_non_retryable() {
        use std::sync::atomic::AtomicBool;
        let config = BackoffConfig {
            max_retries: 1,
            ..Default::default()
        };
        let policy = ProviderRetryPolicy::with_sleeper(config, Box::new(FakeSleeper::new()));
        let cancelled = AtomicBool::new(false);
        let result = retry_with_policy(
            "test",
            &policy,
            || Err::<i32, _>(("HTTP 400".to_string(), StreamState::NoDelta)),
            &cancelled,
        );
        assert!(matches!(result, Err(RetryDecision::Abort { .. })));
    }

    #[test]
    fn retry_with_policy_exhausts_and_escalates() {
        use std::sync::atomic::AtomicBool;
        let config = BackoffConfig {
            max_retries: 3,
            initial_delay_ms: 1,
            ..Default::default()
        };
        let policy = ProviderRetryPolicy::with_sleeper(config, Box::new(FakeSleeper::new()));
        let cancelled = AtomicBool::new(false);
        let tries = std::cell::Cell::new(0);
        let result = retry_with_policy(
            "test",
            &policy,
            || {
                tries.set(tries.get() + 1);
                Err::<i32, _>(("timeout".to_string(), StreamState::NoDelta))
            },
            &cancelled,
        );
        assert!(matches!(result, Err(RetryDecision::Escalate { .. })));
        // max_retries=3 means: 1 initial + up to 3 retries = 4 total calls
        assert_eq!(tries.get(), 4);
    }

    #[test]
    fn retry_after_parse_invalid_returns_none() {
        let ra = RetryAfter::parse("not-a-number");
        assert!(ra.seconds.is_none());
    }

    #[test]
    fn retry_with_policy_cancelled_returns_abort_immediately() {
        use std::sync::atomic::AtomicBool;
        let config = BackoffConfig {
            max_retries: 3,
            ..Default::default()
        };
        let policy = ProviderRetryPolicy::with_sleeper(config, Box::new(FakeSleeper::new()));
        let cancelled = AtomicBool::new(true);
        let tries = std::cell::Cell::new(0);
        let result = retry_with_policy(
            "test",
            &policy,
            || {
                tries.set(tries.get() + 1);
                Err::<i32, _>(("timeout".to_string(), StreamState::NoDelta))
            },
            &cancelled,
        );
        assert!(matches!(result, Err(RetryDecision::Abort { .. })));
        // Should not have called the operation at all
        assert_eq!(tries.get(), 0);
    }

    #[test]
    fn stream_state_transition_policing_prevents_rollback() {
        // Once visible text started, no event should roll back
        let s = StreamState::VisibleTextStarted;
        assert_eq!(
            s.transition(StreamEventKind::Reasoning),
            StreamState::VisibleTextStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::Text),
            StreamState::VisibleTextStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::MixedReasoningText),
            StreamState::VisibleTextStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::ToolCall),
            StreamState::ToolCallStarted
        );
        // Once tool call started, no event should roll back
        let s = StreamState::ToolCallStarted;
        assert_eq!(
            s.transition(StreamEventKind::Reasoning),
            StreamState::ToolCallStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::Text),
            StreamState::ToolCallStarted
        );
        // PreConnection direct to tool call is allowed
        let s = StreamState::PreConnection;
        assert_eq!(
            s.transition(StreamEventKind::ToolCall),
            StreamState::ToolCallStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::Text),
            StreamState::VisibleTextStarted
        );
        // NoDelta text directly jumps to visible text
        let s = StreamState::NoDelta;
        assert_eq!(
            s.transition(StreamEventKind::Text),
            StreamState::VisibleTextStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::MixedReasoningText),
            StreamState::VisibleTextStarted
        );
    }

    #[test]
    fn decide_uses_retry_after_parameter() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::TransientRetryable {
            details: "timeout".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        let ra = RetryAfter { seconds: Some(2) };
        let decision = policy.decide(&failure, &budget, 1, Some(&ra), StreamState::NoDelta);
        // Should still be Retry (retryable + budget available)
        assert!(matches!(decision, RetryDecision::Retry { .. }));
        // Delay should be influenced by RetryAfter
        if let RetryDecision::Retry { delay_ms } = &decision {
            assert!(*delay_ms >= 2000, "retry-after should influence delay");
        }
    }

    #[test]
    fn retry_budget_record_execution() {
        let mut budget = RetryBudget::new(3, 10000);
        assert_eq!(budget.elapsed_ms, 0);
        budget.record_execution(2500);
        assert_eq!(budget.elapsed_ms, 2500);
        budget.record_execution(500);
        assert_eq!(budget.elapsed_ms, 3000);
    }

    #[test]
    fn failure_kind_methods() {
        let u = FailureKind::UnsafeToRetry {
            details: "mid-stream".to_string(),
        };
        assert!(u.is_unsafe_to_retry());
        assert!(!u.requires_request_mutation());
        let m = FailureKind::RequiresRequestMutation {
            details: "context too large".to_string(),
        };
        assert!(m.requires_request_mutation());
        assert!(!m.is_unsafe_to_retry());
    }

    #[test]
    fn jitter_kind_full_edge_values() {
        let cfg = BackoffConfig {
            initial_delay_ms: 1000,
            multiplier: 2.0,
            max_delay_ms: 8000,
            jitter_kind: JitterKind::Full,
            ..Default::default()
        };
        // jitter_sample=0.0 → 0ms delay
        assert_eq!(compute_delay(1, &cfg, 0.0).as_millis(), 0);
        // jitter_sample=1.0 → full delay
        assert_eq!(compute_delay(1, &cfg, 1.0).as_millis(), 1000);
    }

    #[test]
    fn jitter_kind_equal_edge_values() {
        let cfg = BackoffConfig {
            initial_delay_ms: 1000,
            multiplier: 2.0,
            max_delay_ms: 8000,
            jitter_kind: JitterKind::Equal,
            ..Default::default()
        };
        // jitter_sample=0.0 → base/2 = 500ms
        assert_eq!(compute_delay(1, &cfg, 0.0).as_millis(), 500);
        // jitter_sample=1.0 → base = 1000ms
        assert_eq!(compute_delay(1, &cfg, 1.0).as_millis(), 1000);
    }
}
