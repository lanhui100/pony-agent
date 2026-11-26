use std::cell::RefCell;
use std::time::Instant;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderAttemptWindow {
    /// attempt 次数（含最终成功的那次；整体失败的调用计全部尝试次数）。
    pub attempts: u32,
    /// 失败 attempt **自身**耗时之和（毫秒，不含退避 sleep）。
    pub failed_attempt_ms: u64,
    /// 重试循环内的退避 sleep 累计（毫秒）。
    pub slept_ms: u64,
    /// 最后一次**成功** attempt 的耗时（毫秒）；无成功 attempt ⇒ `None`。
    pub succeeded_attempt_ms: Option<u64>,
    /// 最后一次成功 attempt 的**起点**相对账本起点的偏移（毫秒）。
    pub succeeded_attempt_offset_ms: Option<u64>,
}

impl ProviderAttemptWindow {
    /// 成功 attempt 之前累计的耗时（失败尝试 + 退避）。
    pub fn prelude_ms(&self) -> u64 {
        self.failed_attempt_ms.saturating_add(self.slept_ms)
    }

    /// 外层 `dur`（跨 attempt 墙钟）→ 纯净 per-call `dur`（只覆盖最后一次成功 attempt）。
    ///
    /// 无成功 attempt（调用整体失败）⇒ 原样返回外层值，**不伪造**纯净性。
    pub fn rebase_duration_ms(&self, outer_duration_ms: u64) -> u64 {
        self.succeeded_attempt_ms.unwrap_or(outer_duration_ms)
    }

    /// 外层 `ttft`（跨 attempt）→ 纯净 `ttft`（相对成功 attempt 起点）。
    ///
    /// 无成功 attempt ⇒ 原样返回（同上，不伪造）。毫秒级取整可能让结果落到 0
    /// （首 token 在成功 attempt 起点当毫秒内到达）——0 是**诚实值**，不是缺失。
    pub fn rebase_first_token_ms(&self, outer_first_token_ms: Option<u64>) -> Option<u64> {
        outer_first_token_ms.map(|ttft| {
            ttft.saturating_sub(self.succeeded_attempt_offset_ms.unwrap_or(0))
        })
    }
}

/// 账本内部可变状态（读取即消费，一次性）。
#[derive(Debug, Clone, Copy)]
struct AttemptWindowState {
    started_at: Instant,
    attempts: u32,
    failed_attempt_ms: u64,
    slept_ms: u64,
    succeeded_attempt_ms: Option<u64>,
    succeeded_attempt_offset_ms: Option<u64>,
}

impl AttemptWindowState {
    fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            attempts: 0,
            failed_attempt_ms: 0,
            slept_ms: 0,
            succeeded_attempt_ms: None,
            succeeded_attempt_offset_ms: None,
        }
    }

    fn snapshot(&self) -> ProviderAttemptWindow {
        ProviderAttemptWindow {
            attempts: self.attempts,
            failed_attempt_ms: self.failed_attempt_ms,
            slept_ms: self.slept_ms,
            succeeded_attempt_ms: self.succeeded_attempt_ms,
            succeeded_attempt_offset_ms: self.succeeded_attempt_offset_ms,
        }
    }
}

// 为什么用 thread_local：一次 provider 调用（含其内部重试/换手）在**同一线程上同步执行**
// ——`on_delta` 也在阻塞 SSE 读循环内联调用（`provider::openai_sse::push_payload`）。
// 与 `runtime::limits` 的测试覆写同一取舍：不引入跨调用共享状态，也不污染并行用例。
thread_local! {
    static ATTEMPT_WINDOW: RefCell<Option<AttemptWindowState>> = const { RefCell::new(None) };
}

/// 开启一次 provider 调用的重试账本，返回**与外层计时同源**的起点。
///
/// 调用方必须以返回的 `Instant` 作为 `dur` / `ttft` 的计时起点（两者必须同源，
/// 否则 [`ProviderAttemptWindow::rebase_first_token_ms`] 会漂移）。
pub fn begin_provider_attempt_window() -> Instant {
    let started_at = Instant::now();
    ATTEMPT_WINDOW.with(|slot| *slot.borrow_mut() = Some(AttemptWindowState::new(started_at)));
    started_at
}

/// 取走本线程当前调用的账本（一次性；取走后回到「未开启」状态）。
pub fn take_provider_attempt_window() -> Option<ProviderAttemptWindow> {
    ATTEMPT_WINDOW.with(|slot| slot.borrow_mut().take().map(|state| state.snapshot()))
}

/// 窥视本线程当前调用的账本（**不取走**）。
///
/// B135：`take_provider_attempt_window` 是一次性台账——per-call 记录构建已取走它，
/// 之后到来的 `llm.chat` span 记账就只剩外层（含重试等待）耗时。本函数提供只读视图，
/// 让「记录构建」与「span 记账」共享同一份窗口，而不改变任一方的取走语义。
pub fn peek_provider_attempt_window() -> Option<ProviderAttemptWindow> {
    ATTEMPT_WINDOW.with(|slot| slot.borrow().as_ref().map(AttemptWindowState::snapshot))
}

/// 重基后的耗时三元组（B135）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RebasedTiming {
    /// 只覆盖最后一次成功 attempt 的耗时；无账本/无成功 attempt ⇒ 原样返回。
    pub duration_ms: Option<u64>,
    /// 同样重基后的首包延迟。
    pub first_token_ms: Option<u64>,
    /// 被排除掉的重试等待（失败 attempt 自身耗时 + 退避 sleep）。
    /// **无重试 ⇒ `None`**（键缺席，禁止 0 占位冒充真值）。
    pub retry_wait_ms: Option<u64>,
}

/// 把外层（含重试等待）耗时重基到「最后一次成功 attempt」的纯窗口。
///
/// 语义与 `take_pure_call_window` 一致，但**不消费账本**；供 `llm.chat` span 侧使用。
pub fn rebase_timing_to_pure_call(
    duration_ms: Option<u64>,
    first_token_ms: Option<u64>,
) -> RebasedTiming {
    let Some(window) = peek_provider_attempt_window() else {
        return RebasedTiming {
            duration_ms,
            first_token_ms,
            retry_wait_ms: None,
        };
    };
    let rebased_duration = duration_ms.map(|outer| window.rebase_duration_ms(outer));
    let retry_wait_ms = match (duration_ms, rebased_duration) {
        (Some(outer), Some(pure)) if pure < outer => Some(outer - pure),
        _ => None,
    };
    RebasedTiming {
        duration_ms: rebased_duration,
        first_token_ms: window.rebase_first_token_ms(first_token_ms),
        retry_wait_ms,
    }
}

/// 记一次**失败** attempt（供重试循环调用；未开启账本 ⇒ 静默 no-op）。
pub fn note_provider_attempt_failed(elapsed_ms: u64) {
    ATTEMPT_WINDOW.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.attempts = state.attempts.saturating_add(1);
            state.failed_attempt_ms = state.failed_attempt_ms.saturating_add(elapsed_ms);
        }
    });
}

/// 记一次**成功** attempt（供重试循环调用；未开启账本 ⇒ 静默 no-op）。
///
/// `started_at` 是本次 attempt 的起点：由它算出成功 attempt 相对账本起点的偏移，
/// 从而把外层 `ttft` 平移到「成功 attempt 内部」。
pub fn note_provider_attempt_succeeded(started_at: Instant, elapsed_ms: u64) {
    ATTEMPT_WINDOW.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.attempts = state.attempts.saturating_add(1);
            state.succeeded_attempt_ms = Some(elapsed_ms);
            state.succeeded_attempt_offset_ms = Some(
                started_at
                    .saturating_duration_since(state.started_at)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
            );
        }
    });
}

/// 记重试循环内一次退避 sleep 的时长（未开启账本 ⇒ 静默 no-op）。
pub fn note_provider_attempt_backoff_slept(slept_ms: u64) {
    ATTEMPT_WINDOW.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.slept_ms = state.slept_ms.saturating_add(slept_ms);
        }
    });
}
