//! W33 验收测试：平台化收尾 —— 真实 USD 成本估算、用量核算与最大预算硬熔断（Cost Tracker & Budget Breaker）
//!
//! 契约要求：
//! 1. `CostTracker` 支持记录 token 输入、输出及缓存命中数量，并计算精确 USD 成本。
//! 2. 支持 `max_budget_usd` 硬熔断保护：当会话累计花费超过预算上限时，触发 `budget_exceeded` 熔断错误。
//! 3. 支持针对不同模型定价矩阵的成本统计。
//! 4. 提供结构化 `/cost` 或会话成本摘要，包含总 token 数、缓存节省以及 USD 金额。

use pony_agent_core::agent::budget::{CostTracker, ModelPricing};

#[test]
fn test_cost_tracker_calculation_and_budget_breaker() {
    let mut tracker = CostTracker::new(Some(0.05)); // 预算上限 0.05 USD
    let pricing = ModelPricing {
        input_per_million_usd: 3.0,
        output_per_million_usd: 15.0,
        cached_input_per_million_usd: 0.75,
    };

    // 1. 记录第一次调用：未超预算
    let cost1 = tracker.record_usage("claude-3-7-sonnet", &pricing, 10_000, 1_000, 5_000).expect("within budget");
    // input: 5000 * 3.0 / 1M = 0.015, cached: 5000 * 0.75 / 1M = 0.00375, output: 1000 * 15.0 / 1M = 0.015
    // cost1 approx = 0.03375
    assert!(cost1 > 0.03 && cost1 < 0.04);
    assert!(!tracker.is_budget_exceeded());

    // 2. 记录第二次调用：累加后触发 0.05 USD 硬熔断
    let err = tracker.record_usage("claude-3-7-sonnet", &pricing, 10_000, 2_000, 0).expect_err("should exceed budget");
    assert!(err.contains("budget_exceeded") || err.contains("Budget exceeded"));
    assert!(tracker.is_budget_exceeded());

    // 3. 统计结果完整性验证
    let summary = tracker.summary();
    assert!(summary.total_tokens > 20_000);
    assert!(summary.total_cost_usd > 0.05);
    assert_eq!(summary.currency, "USD");
}
