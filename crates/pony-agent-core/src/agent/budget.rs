//! Atomic budget ledger and cancellation token for governed tool dispatch (PA-076 phase 3).
//!
//! A `BudgetLedger` is created once per top-level invocation and shared (via `Arc`) by every
//! child that runs underneath it. All reservations are atomic so composite handlers dispatching
//! children from multiple threads cannot oversubscribe a shared budget. Every reservation path
//! fails closed: `reserve_call`, `begin_execution`, `reserve_bytes`, and `check_deadline` return
//! structured errors instead of silently degrading.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// A shared cancellation token. Once cancelled, every dependent invocation fails closed with a
/// `Cancelled` execution status instead of producing a provider-consumable result.
#[derive(Debug, Default)]
pub struct CancellationToken {
    cancelled: AtomicBool,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn check(&self) -> Result<(), String> {
        if self.is_cancelled() {
            Err("tool invocation cancelled".to_string())
        } else {
            Ok(())
        }
    }
}

/// Tunable budget limits for governed dispatch. A value of `0` means "unlimited" for the
/// corresponding numeric limit (except `max_composite_depth`, where `0` disables child
/// dispatch entirely).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchBudgetConfig {
    /// Maximum number of child invocations allowed under one top-level invocation.
    pub max_composite_calls: u64,
    /// Maximum number of concurrently executing children (`0` = unlimited).
    pub max_concurrency: u64,
    /// Maximum composite nesting depth (`0` = no children allowed).
    pub max_composite_depth: u32,
    /// Global output-byte cap for an invocation. `0` falls back to the descriptor
    /// execution-policy `result_budget_bytes`.
    pub max_result_bytes: u64,
    /// Default time-to-live for persisted control requests, in milliseconds.
    pub control_request_expiry_ms: u64,
    /// Legacy-compatibility mode: ignore the descriptor's output-byte and deadline budgets and
    /// run every invocation unbounded (no `output_budget_exceeded`, no `deadline exceeded`).
    /// Used by `build_governed_executor` so the runtime switch reproduces the legacy
    /// `ToolRouter`'s absence of tool-level byte/deadline caps (phase-4..7 review P1-2), instead
    /// of silently regressing large reads/batches.
    pub unbounded_output_and_deadline: bool,
}

impl Default for DispatchBudgetConfig {
    fn default() -> Self {
        Self {
            max_composite_calls: 24,
            max_concurrency: 0,
            max_composite_depth: 4,
            max_result_bytes: 0,
            control_request_expiry_ms: 60_000,
            unbounded_output_and_deadline: false,
        }
    }
}

/// Atomically reserved budget ledger shared by one top-level invocation and every child that
/// runs underneath it.
#[derive(Debug)]
pub struct BudgetLedger {
    max_calls: u64,
    calls_reserved: AtomicU64,
    max_concurrency: u64,
    concurrency_active: AtomicU64,
    max_bytes: u64,
    bytes_reserved: AtomicU64,
    start_ms: u64,
    max_duration_ms: u64,
}

impl BudgetLedger {
    pub fn new(
        max_calls: u64,
        max_concurrency: u64,
        max_bytes: u64,
        start_ms: u64,
        max_duration_ms: u64,
    ) -> Self {
        Self {
            max_calls,
            calls_reserved: AtomicU64::new(0),
            max_concurrency,
            concurrency_active: AtomicU64::new(0),
            max_bytes,
            bytes_reserved: AtomicU64::new(0),
            start_ms,
            max_duration_ms,
        }
    }

    /// Atomically reserve one call slot. Fails closed when the call budget is exhausted.
    pub fn reserve_call(&self) -> Result<(), String> {
        if self.max_calls == 0 {
            return Ok(());
        }
        let current = self.calls_reserved.fetch_add(1, Ordering::Relaxed);
        if current >= self.max_calls {
            self.calls_reserved.fetch_sub(1, Ordering::Relaxed);
            return Err(format!("call budget exhausted (max {})", self.max_calls));
        }
        Ok(())
    }

    /// Atomically acquire a concurrency slot. The returned guard releases the slot on drop, so
    /// a panicking handler cannot leak the reservation.
    pub fn begin_execution(self: &Arc<Self>) -> Result<ConcurrencyGuard, String> {
        if self.max_concurrency != 0 {
            let current = self.concurrency_active.fetch_add(1, Ordering::Relaxed);
            if current >= self.max_concurrency {
                self.concurrency_active.fetch_sub(1, Ordering::Relaxed);
                return Err(format!(
                    "concurrency budget exhausted (max {})",
                    self.max_concurrency
                ));
            }
            return Ok(ConcurrencyGuard {
                ledger: Arc::clone(self),
                active: true,
            });
        }
        Ok(ConcurrencyGuard {
            ledger: Arc::clone(self),
            active: false,
        })
    }

    fn release_concurrency(&self) {
        self.concurrency_active.fetch_sub(1, Ordering::Relaxed);
    }

    /// Atomically reserve `bytes` of output budget. Fails closed when the byte budget is
    /// exhausted.
    pub fn reserve_bytes(&self, bytes: u64) -> Result<(), String> {
        if self.max_bytes == 0 {
            return Ok(());
        }
        let current = self.bytes_reserved.fetch_add(bytes, Ordering::Relaxed);
        if current.saturating_add(bytes) > self.max_bytes {
            self.bytes_reserved.fetch_sub(bytes, Ordering::Relaxed);
            return Err(format!(
                "output budget exhausted (max {} bytes, tried to add {bytes})",
                self.max_bytes
            ));
        }
        Ok(())
    }

    /// Fails closed once `now_ms` passes the invocation deadline.
    pub fn check_deadline(&self, now_ms: u64) -> Result<(), String> {
        if self.max_duration_ms == 0 {
            return Ok(());
        }
        let deadline = self.start_ms.saturating_add(self.max_duration_ms);
        if now_ms > deadline {
            Err(format!(
                "deadline exceeded at {} ms (started at {}, budget {} ms)",
                now_ms, self.start_ms, self.max_duration_ms
            ))
        } else {
            Ok(())
        }
    }

    pub fn deadline_absolute_ms(&self) -> u64 {
        self.start_ms.saturating_add(self.max_duration_ms)
    }

    /// Remaining time until the deadline, or `None` when the invocation has no duration limit.
    pub fn remaining_deadline_ms(&self, now_ms: u64) -> Option<u64> {
        if self.max_duration_ms == 0 {
            return None;
        }
        Some(self.deadline_absolute_ms().saturating_sub(now_ms))
    }

    pub fn calls_reserved(&self) -> u64 {
        self.calls_reserved.load(Ordering::Relaxed)
    }

    pub fn bytes_reserved(&self) -> u64 {
        self.bytes_reserved.load(Ordering::Relaxed)
    }
}

/// RAII guard that releases a concurrency slot when dropped.
#[derive(Debug)]
pub struct ConcurrencyGuard {
    ledger: Arc<BudgetLedger>,
    active: bool,
}

impl Drop for ConcurrencyGuard {
    fn drop(&mut self) {
        // Only decrement when a slot was actually acquired. With `max_concurrency == 0`
        // (unlimited) `begin_execution` returns a non-counting guard and release must be a no-op,
        // otherwise the shared counter underflows to `u64::MAX` (phase-3 review P3).
        if self.active {
            self.ledger.release_concurrency();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_token_checks_after_cancel() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        assert!(token.check().is_ok());
        token.cancel();
        assert!(token.is_cancelled());
        assert!(token.check().is_err());
    }

    #[test]
    fn call_budget_exhaustion_is_atomic_and_fail_closed() {
        let ledger = Arc::new(BudgetLedger::new(2, 0, 0, 100, 0));
        assert!(ledger.reserve_call().is_ok());
        assert!(ledger.reserve_call().is_ok());
        assert!(ledger.reserve_call().is_err());
        assert_eq!(ledger.calls_reserved(), 2);
    }

    #[test]
    fn concurrency_guard_releases_on_drop() {
        let ledger = Arc::new(BudgetLedger::new(0, 1, 0, 100, 0));
        {
            let _guard = ledger.begin_execution().expect("first slot is free");
            assert!(ledger.begin_execution().is_err());
        }
        // The guard dropped, so a new execution can acquire the slot.
        assert!(ledger.begin_execution().is_ok());
    }

    #[test]
    fn byte_and_deadline_budgets_fail_closed() {
        let ledger = BudgetLedger::new(0, 0, 10, 100, 50);
        assert!(ledger.reserve_bytes(6).is_ok());
        assert!(ledger.reserve_bytes(4).is_ok());
        assert!(ledger.reserve_bytes(1).is_err());
        assert!(ledger.check_deadline(149).is_ok());
        assert!(ledger.check_deadline(151).is_err());
        assert_eq!(ledger.remaining_deadline_ms(120), Some(30));
    }
}
