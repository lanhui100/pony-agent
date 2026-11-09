//! PA-172 **逐 attempt 超时钳制**机械锚点（矩阵 §3.2 M3 / §3.3 F-c）。
//!
//! 契约真值源：`.dev-team/contract-matrix-pa172.md`（v2 定稿）
//! - §3.1「`FAILOVER_BUDGET_MS` 是**逐 attempt deadline**，不是换手前查一次的闸门：
//!   每次候选 attempt 前重算 remaining，per-attempt 超时 = `min(requestTimeoutMs, remaining)`；
//!   remaining ≤ 0 ⇒ 立即 exhausted」；
//! - §3.2 **M3**：该钳制必须是可机判的**纯函数**（审查 round2 指出「per-attempt 钳制不可机判」，
//!   Lead 裁决补此锚点）。
//!
//! 冻结签名：
//! ```rust,ignore
//! // pony_agent_core::agent::retry
//! pub fn failover_attempt_timeout_ms(policy_timeout_ms: u64, remaining_ms: u64) -> u64; // == min(..)
//! ```
//!
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。
//!
//! 红相预期（wave-69 实现前）：该函数不存在 ⇒ 编译红（E0432），属冻结签名锚点的合法红相。
//! 覆盖点除 min 语义外，还钉住「零剩余」「相等边界」「`u64::MAX` 不得溢出/饱和成错值」三类
//! 易错实现面（`a.min(b)` 与 `saturating_sub` 混淆、`as u32` 截断等）。

use pony_agent_core::agent::config::UPSTREAM_REQUEST_TIMEOUT_MS_DEFAULT;
use pony_agent_core::agent::provider::FAILOVER_BUDGET_MS;
use pony_agent_core::agent::retry::failover_attempt_timeout_ms;

/// §3.3 F-c：`remaining` 仍大于策略超时 ⇒ 单次 attempt 取**策略超时**（不得被预算放大）。
#[test]
fn clamp_takes_policy_when_remaining_larger() {
    // Arrange
    let policy_timeout_ms = 45_000_u64;
    let remaining_ms = 120_000_u64;

    // Act
    let actual = failover_attempt_timeout_ms(policy_timeout_ms, remaining_ms);

    // Assert
    assert_eq!(
        actual, 45_000,
        "F-c：remaining({remaining_ms}) > policy({policy_timeout_ms}) ⇒ 必须取 {policy_timeout_ms}，实际 {actual}"
    );
    assert!(
        actual <= FAILOVER_BUDGET_MS,
        "F-c：单次 attempt 超时不得超过换手总预算 {FAILOVER_BUDGET_MS}ms，实际 {actual}"
    );
}

/// §3.3 F-c：预算剩余不足 ⇒ 单次 attempt 取**剩余预算**（不得让候选再跑满 45s）。
#[test]
fn clamp_takes_remaining_when_smaller() {
    // Arrange
    let policy_timeout_ms = 45_000_u64;
    let remaining_ms = 3_000_u64;

    // Act
    let actual = failover_attempt_timeout_ms(policy_timeout_ms, remaining_ms);

    // Assert
    assert_eq!(
        actual, 3_000,
        "F-c：remaining({remaining_ms}) < policy({policy_timeout_ms}) ⇒ 必须取 {remaining_ms}，实际 {actual}"
    );
}

/// §3.3 F-c：剩余预算为 0 ⇒ attempt 超时必须是 0（调用方据此**立即** exhausted，不得再发 HTTP）。
#[test]
fn clamp_zero_remaining_is_zero() {
    // Arrange
    let policy_timeout_ms = 45_000_u64;

    // Act
    let actual = failover_attempt_timeout_ms(policy_timeout_ms, 0);

    // Assert
    assert_eq!(
        actual, 0,
        "F-c：remaining=0 ⇒ 必须返回 0（语义 = 立即停止换手走 exhausted），实际 {actual}"
    );
}

/// 边界：`policy == remaining` ⇒ 取该值；极小值不得被任何「最小 1ms」类兜底改写。
#[test]
fn clamp_boundary_equal() {
    assert_eq!(
        failover_attempt_timeout_ms(5_000, 5_000),
        5_000,
        "F-c：policy == remaining == 5000 ⇒ 必须返回 5000"
    );
    assert_eq!(
        failover_attempt_timeout_ms(1, 1),
        1,
        "F-c：极小值 (1, 1) ⇒ 必须返回 1（不得被最小兜底改写）"
    );
}

/// 溢出钉子：`policy = u64::MAX` 时取较小的 remaining，**不得**溢出/截断/饱和成错值。
#[test]
fn clamp_huge_policy_does_not_overflow() {
    // Arrange + Act
    let actual = failover_attempt_timeout_ms(u64::MAX, 120_000);

    // Assert
    assert_eq!(
        actual, 120_000,
        "F-c：policy=u64::MAX, remaining=120000 ⇒ 必须返回 120000（防溢出/饱和实现错误），实际 {actual}"
    );
    assert_eq!(
        failover_attempt_timeout_ms(u64::MAX, u64::MAX),
        u64::MAX,
        "F-c：两侧同为 u64::MAX ⇒ 必须返回 u64::MAX（不得 0 化或饱和成别的值）"
    );
    assert_eq!(
        failover_attempt_timeout_ms(0, 0),
        0,
        "F-c：(0, 0) ⇒ 必须返回 0"
    );
}

/// 与默认值/预算常量联动：默认策略 45s 在总预算 120s 内 ⇒ 首跳 attempt 仍取 45s（G2 默认零行为变更）。
#[test]
fn policy_timeout_default_is_45s() {
    // Arrange + Act
    let actual = failover_attempt_timeout_ms(
        UPSTREAM_REQUEST_TIMEOUT_MS_DEFAULT,
        FAILOVER_BUDGET_MS,
    );

    // Assert
    assert_eq!(
        UPSTREAM_REQUEST_TIMEOUT_MS_DEFAULT, 45_000,
        "G2：默认请求总超时冻结为 45000ms"
    );
    assert_eq!(
        FAILOVER_BUDGET_MS, 120_000,
        "F-c：换手总预算冻结为 120000ms"
    );
    assert_eq!(
        actual, 45_000,
        "F-c/G2：默认 policy(45s) 小于预算(120s) ⇒ 首跳 attempt 必须仍是 45s，实际 {actual}"
    );
}
