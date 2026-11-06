//! B027 验收（L2 契约级）：成本协议口径统一 —— wire / 落库 / budget 三面同源。
//!
//! 冻结契约（`.dev-team/contract-matrix-cost-protocol.md`）：
//! - C1 单一出口：所有成本计算都经 `cost::estimate_cost_with_protocol` / 其包装，
//!   `protocol` 由 `ProviderEventMeta.protocol` → `ProviderCostProtocol::from_label` 映射。
//! - C2 协议归一：`Anthropic` ⇒ `uncached = input`（input 不含 cache，禁止相减）；
//!   `OpenAi` / `Unknown` ⇒ `uncached = input.saturating_sub(cache)`。
//! - C3 wire 与落库同值：两者都由 `turn_flow::estimate_call_cost_usd` 单函数产出。
//! - C4 budget 自洽：`total_tokens == total_input_tokens + total_cached_tokens + total_output_tokens`，
//!   且三者的口径与 C2 一致（`total_input_tokens` = 协议归一后的未缓存输入）。
//! - C5 未知不伪造：未收录模型 / 缺 usage ⇒ `None`（禁止 0 顶替）。

use pony_agent_core::agent::budget::{CostTracker, ModelPricing as BudgetPricing};
use pony_agent_core::agent::cost::{
    estimate_cost, estimate_cost_with_protocol, normalized_uncached_input_tokens,
    ProviderCostProtocol,
};
use pony_agent_core::agent::turn_flow::{estimate_call_cost_usd, ProviderEventMeta};

const MODEL: &str = "claude-3-5-sonnet";

fn meta(protocol: &str) -> ProviderEventMeta {
    ProviderEventMeta {
        requested_name: "test-provider".to_string(),
        provider_name: "test-provider".to_string(),
        protocol: protocol.to_string(),
        model: MODEL.to_string(),
        context_window_tokens: Some(200_000),
    }
}

fn budget_pricing() -> BudgetPricing {
    // 与 `cost::PRICING_TABLE` 的 claude-3-5-sonnet 同值（$3 / $15 / cache-read $0.30 per Mtok）。
    BudgetPricing {
        input_per_million_usd: 3.0,
        output_per_million_usd: 15.0,
        cached_input_per_million_usd: 0.30,
    }
}

// ---------------------------------------------------------------------------
// C2：协议归一后的未缓存输入
// ---------------------------------------------------------------------------
#[test]
fn c2_normalized_uncached_input_anthropic_keeps_input() {
    // Anthropic：input 不含 cache_read ⇒ 未缓存输入就是 input 本身（禁止相减）。
    assert_eq!(
        normalized_uncached_input_tokens(1_000, Some(8_000), ProviderCostProtocol::Anthropic),
        1_000,
        "Anthropic 下未缓存输入必须等于 input（相减会把输入费清零）"
    );
}

#[test]
fn c2_normalized_uncached_input_openai_subtracts_cache() {
    // OpenAI / Unknown：input 已含 cache ⇒ 需剔除。
    assert_eq!(
        normalized_uncached_input_tokens(1_000, Some(8_000), ProviderCostProtocol::OpenAi),
        0,
        "OpenAI 下 input < cache 时必须饱和到 0，不得下溢"
    );
    assert_eq!(
        normalized_uncached_input_tokens(10_000, Some(8_000), ProviderCostProtocol::OpenAi),
        2_000
    );
    assert_eq!(
        normalized_uncached_input_tokens(10_000, None, ProviderCostProtocol::Unknown),
        10_000
    );
}

// ---------------------------------------------------------------------------
// C1 + C3：wire 与落库同源，且协议感知
// ---------------------------------------------------------------------------
#[test]
fn c3_call_cost_is_protocol_aware_anthropic_vs_openai() {
    // input=1000, cache_read=8000, output=500
    // Anthropic: (1000*3 + 8000*0.30)/1e6 + 500*15/1e6 = 0.0054 + 0.0075 = 0.0129
    // OpenAI   : (0*3   + 8000*0.30)/1e6 + 500*15/1e6 = 0.0024 + 0.0075 = 0.0099
    let anthropic = estimate_call_cost_usd(Some(&meta("anthropic-messages")), Some(1_000), Some(500), Some(8_000))
        .expect("anthropic 成本必须可算");
    let openai = estimate_call_cost_usd(Some(&meta("openai-completions")), Some(1_000), Some(500), Some(8_000))
        .expect("openai 成本必须可算");

    assert!(
        (anthropic - 0.0129).abs() < 1e-9,
        "anthropic 成本应为 0.0129，实际 {anthropic}"
    );
    assert!(
        (openai - 0.0099).abs() < 1e-9,
        "openai 成本应为 0.0099，实际 {openai}"
    );
    assert!(
        anthropic > openai,
        "cache 大于 input 时 anthropic 不得低于 openai（旧实现按 OpenAI 口径相减会低估）"
    );
}

#[test]
fn c3_call_cost_equals_protocol_primitive_and_differs_from_legacy() {
    let input = 1_000u64;
    let output = 500u64;
    let cache = Some(8_000u64);

    let via_outlet = estimate_call_cost_usd(Some(&meta("anthropic-messages")), Some(input), Some(output), cache)
        .expect("outlet 必须给出成本");
    let via_primitive =
        estimate_cost_with_protocol(MODEL, input, output, cache, ProviderCostProtocol::Anthropic)
            .expect("primitive 必须给出成本")
            .total_cost_usd;
    assert_eq!(
        via_outlet, via_primitive,
        "wire/落库出口必须与协议原语逐位一致（同一函数）"
    );

    // 旧实现（固定 OpenAI 口径）会低估：证明该差异确实存在，防止回归到 legacy 入口。
    let legacy = estimate_cost(MODEL, input, output, cache)
        .expect("legacy 入口必须仍可算")
        .total_cost_usd;
    assert!(
        via_outlet > legacy,
        "anthropic 单出口成本 {via_outlet} 必须高于 legacy(OpenAI) 口径 {legacy}"
    );
}

#[test]
fn c5_unknown_model_or_missing_usage_yields_none() {
    let unknown_model = ProviderEventMeta {
        model: "definitely-not-priced-model".to_string(),
        ..meta("anthropic-messages")
    };
    assert_eq!(
        estimate_call_cost_usd(Some(&unknown_model), Some(1_000), Some(500), None),
        None,
        "未收录模型必须返回 None，禁止 0 伪造"
    );
    assert_eq!(
        estimate_call_cost_usd(Some(&meta("anthropic-messages")), Some(1_000), None, None),
        None,
        "缺 output usage 必须返回 None"
    );
    assert_eq!(
        estimate_call_cost_usd(None, Some(1_000), Some(500), None),
        None,
        "无 provider_meta 必须返回 None（无法判定协议）"
    );
}

// ---------------------------------------------------------------------------
// C4：budget 汇总自洽（分项和 == total，两种协议都成立）
// ---------------------------------------------------------------------------
#[test]
fn c4_budget_summary_self_consistent_for_anthropic() {
    let mut tracker = CostTracker::new(None);
    // Anthropic：input=1000(不含 cache), cache_read=8000, output=500
    tracker
        .record_usage(
            MODEL,
            &budget_pricing(),
            1_000,
            500,
            8_000,
            ProviderCostProtocol::Anthropic,
        )
        .expect("无预算上限时不得失败");

    let s = tracker.summary();
    assert_eq!(s.total_input_tokens, 1_000, "anthropic 未缓存输入 = input");
    assert_eq!(s.total_cached_tokens, 8_000);
    assert_eq!(s.total_output_tokens, 500);
    assert_eq!(
        s.total_tokens,
        s.total_input_tokens + s.total_cached_tokens + s.total_output_tokens,
        "C4：分项和必须等于 total（anthropic）"
    );
    assert!(
        (s.total_cost_usd - 0.0129).abs() < 1e-9,
        "anthropic 汇总成本应为 0.0129，实际 {}",
        s.total_cost_usd
    );
}

#[test]
fn c4_budget_summary_self_consistent_for_openai() {
    let mut tracker = CostTracker::new(None);
    // OpenAI：input=10000(已含 cache), cache_read=8000, output=500
    tracker
        .record_usage(
            MODEL,
            &budget_pricing(),
            10_000,
            500,
            8_000,
            ProviderCostProtocol::OpenAi,
        )
        .expect("无预算上限时不得失败");

    let s = tracker.summary();
    assert_eq!(s.total_input_tokens, 2_000, "openai 未缓存输入 = input - cache");
    assert_eq!(s.total_cached_tokens, 8_000);
    assert_eq!(s.total_output_tokens, 500);
    assert_eq!(
        s.total_tokens,
        s.total_input_tokens + s.total_cached_tokens + s.total_output_tokens,
        "C4：分项和必须等于 total（openai）"
    );
}

#[test]
fn c4_budget_multi_call_accumulates_consistently() {
    let mut tracker = CostTracker::new(None);
    tracker
        .record_usage(MODEL, &budget_pricing(), 1_000, 500, 8_000, ProviderCostProtocol::Anthropic)
        .expect("call 1");
    tracker
        .record_usage(MODEL, &budget_pricing(), 2_000, 100, 0, ProviderCostProtocol::Anthropic)
        .expect("call 2");

    let s = tracker.summary();
    assert_eq!(s.total_input_tokens, 3_000);
    assert_eq!(s.total_cached_tokens, 8_000);
    assert_eq!(s.total_output_tokens, 600);
    assert_eq!(
        s.total_tokens,
        s.total_input_tokens + s.total_cached_tokens + s.total_output_tokens
    );
}

// ---------------------------------------------------------------------------
// C1 结构护栏：防止任何一面回退到 legacy 固定口径入口
// ---------------------------------------------------------------------------
#[test]
fn c1_no_surface_falls_back_to_legacy_openai_pricing() {
    let turn_persist = include_str!("../src/agent/runtime/turn_persist.rs");
    assert!(
        !turn_persist.contains("estimate_cost("),
        "落库路径不得再调用固定 OpenAI 口径的 cost::estimate_cost("
    );
    assert!(
        turn_persist.contains("estimate_call_cost_usd("),
        "落库路径必须委托 turn_flow::estimate_call_cost_usd（单一出口）"
    );

    let budget = include_str!("../src/agent/budget.rs");
    assert!(
        !budget.contains("calculate_cost_with_pricing("),
        "budget 不得再调用固定 OpenAI 口径的 calculate_cost_with_pricing("
    );
    assert!(
        budget.contains("calculate_cost_with_protocol("),
        "budget 必须使用协议感知的 calculate_cost_with_protocol("
    );
    assert!(
        budget.contains("normalized_uncached_input_tokens("),
        "budget 的未缓存输入必须取自 cost 的单一真值函数"
    );

    let turn_flow = include_str!("../src/agent/turn_flow.rs");
    assert!(
        turn_flow.contains("pub fn estimate_call_cost_usd"),
        "wire/落库共享出口 estimate_call_cost_usd 必须存在"
    );
}
