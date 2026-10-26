//! 会话成本估算（Wave-5 K1/K3 唯一计价真源）。
//!
//! 口径（契约 `.dev-team/contracts/wave-5-cost.md` K1）：
//! - 价格以**每 1M token 的 USD 单价**表达，三维阶梯：未缓存输入 / 输出 / 缓存读取；
//! - `input_tokens_uncached = max(input_tokens − cache_read_tokens, 0)`（严禁双重计费）；
//! - 结果四舍五入保留 [`COST_PRECISION_DECIMALS`] 位小数；
//! - **未知模型返回 `None`**：严禁兜底价、插值或猜测；
//! - `NaN` / `±Infinity` / 负单价 ⇒ `None`。
//!
//! 价格表原则（K1 边界 + Lead 裁决）：**只收录可公开核价的主流模型**；
//! 本仓库里使用虚构型号的（`gpt-5.5`、`gpt-5-alpha`、`gemini-3.8-flash`、`deepseek-v4-flash` 等）
//! **一律不收录**（⇒ 计价为 `None`，前端显示「—」），这比编造价格更安全。
//! 供应商改价后需同步本表；无法确认时请**删除条目**而不是猜。

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// 成本精度：USD 小数位（与 NFR `cost_precision_decimals = 6` 对齐）。
pub const COST_PRECISION_DECIMALS: u32 = 6;

/// 每 1M token 的换算分母。
const TOKENS_PER_MTOK: f64 = 1_000_000.0;

/// 四舍五入到 6 位小数的乘数。
const ROUND_SCALE: f64 = 1_000_000.0;

/// 单模型三维阶梯单价（USD / 1M tokens）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelPricing {
    /// 未命中缓存的输入 token 单价。
    pub input_per_mtok: f64,
    /// 输出 token 单价。
    pub output_per_mtok: f64,
    /// 命中 prompt/context cache 的输入 token 单价（读取侧）。
    pub cache_read_per_mtok: f64,
}

/// 一次计价的成本拆分（USD，已按 6 位小数取整）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostEstimate {
    /// 输入侧成本（未缓存 + 缓存读取）。
    pub input_cost_usd: f64,
    /// 输出侧成本。
    pub output_cost_usd: f64,
    /// 合计成本。
    pub total_cost_usd: f64,
}

/// 计价口径所需的「协议语义类别」（Wave-6 K1.1）。
///
/// 只描述**语义差异**，不依赖 provider 模块类型（保持 `cost` 为纯函数模块）：
/// - [`ProviderCostProtocol::OpenAi`]：`prompt_tokens` **已包含**缓存命中 token
///   ⇒ `uncached = max(input − cache_read, 0)`；
/// - [`ProviderCostProtocol::Anthropic`]：`input_tokens` **不含**缓存读取 token
///   ⇒ `uncached = input`（缓存部分独立按 `cache_read_per_mtok` 计费）；
/// - [`ProviderCostProtocol::Unknown`]：保守**按 OpenAI 语义**处理。
///
/// ⚠️ 风险周知（契约 K1.1.3）：若把 Anthropic 兼容上游误标为 Unknown/OpenAI，
/// 缓存场景下会**轻微少计**输入成本（不会多收）；因此所有接入点必须显式传入准确协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCostProtocol {
    OpenAi,
    Anthropic,
    Unknown,
}

impl ProviderCostProtocol {
    /// 由 provider 协议标签映射（`"anthropic-messages"` / `"openai-completions"` / …）。
    ///
    /// 语义：大小写不敏感；无法识别 ⇒ [`ProviderCostProtocol::Unknown`]（保守按 OpenAI 口径）。
    pub fn from_label(label: &str) -> Self {
        let normalized = label.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "" => Self::Unknown,
            // Anthropic 原生协议族（含 Bedrock Claude / 各类 anthropic 兼容标签）。
            "anthropic" | "anthropic-messages" | "anthropic_messages" | "bedrock"
            | "bedrock-anthropic" | "claude" => Self::Anthropic,
            // OpenAI 协议族（含 DeepSeek / Azure / Ollama / 各类 openai 兼容标签）。
            "openai" | "openai-completions" | "openai_completions" | "openai-responses"
            | "openai_responses" | "openai-chat" | "chat-completions" | "deepseek" | "azure"
            | "ollama" => Self::OpenAi,
            _ => Self::Unknown,
        }
    }
}

// ------------------------------------------------------------------ 价格表（公开定价）
//
// 价格快照日期：2026-10-25；单位 USD / 1M tokens；变更需同步 docs 并在 PR 描述附来源链接。
//
// 收录标准：官方公开价格页可核价、且被本仓库默认模型/常见配置实际引用的型号。
// 单价单位统一换算为 USD / 1M tokens；缓存读取价按官方 prompt-caching 折扣价填写。
//
// 「宁缺勿错」取舍（Lead 批准的显式取舍）：**未收录 ⇒ 计价 `None`**，前端显示「—」。
// 绝不为了「表看起来完整」补入无法核价的型号（如本仓库虚构的 `gpt-5.5` / `gpt-5-alpha` /
// `gemini-3.8-flash` / `deepseek-v4-flash`，以及没有稳定公开价的 o1/o3/claude-4* 等）——
// 错误价格会直接误导用户的钱包判断，缺失价格只是少一条展示。
const PRICING_TABLE: &[(&str, ModelPricing)] = &[
    // Anthropic Claude 3.5 Sonnet（官方公开价：$3 / $15，cache read $0.30 = 0.1×input）。
    // 来源：https://www.anthropic.com/pricing（2026-10-25 快照）
    (
        "claude-3-5-sonnet",
        ModelPricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.30,
        },
    ),
    (
        "claude-3-5-sonnet-latest",
        ModelPricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.30,
        },
    ),
    (
        "claude-3-5-sonnet-20241022",
        ModelPricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.30,
        },
    ),
    (
        "claude-3-5-sonnet-20240620",
        ModelPricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.30,
        },
    ),
    // Anthropic Claude 3.7 Sonnet（官方公开价：$3 / $15，cache read $0.30）。
    // 来源：https://www.anthropic.com/pricing（2026-10-25 快照）
    (
        "claude-3-7-sonnet",
        ModelPricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.30,
        },
    ),
    (
        "claude-3-7-sonnet-latest",
        ModelPricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.30,
        },
    ),
    // Anthropic Claude 3.5 Haiku（官方公开价：$0.80 / $4，cache read $0.08）。
    // 来源：https://www.anthropic.com/pricing（2026-10-25 快照）
    (
        "claude-3-5-haiku",
        ModelPricing {
            input_per_mtok: 0.80,
            output_per_mtok: 4.0,
            cache_read_per_mtok: 0.08,
        },
    ),
    (
        "claude-3-5-haiku-latest",
        ModelPricing {
            input_per_mtok: 0.80,
            output_per_mtok: 4.0,
            cache_read_per_mtok: 0.08,
        },
    ),
    // Anthropic Claude 3 Opus（官方公开价：$15 / $75，cache read $1.50）。
    // 来源：https://www.anthropic.com/pricing（2026-10-25 快照）
    (
        "claude-3-opus",
        ModelPricing {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            cache_read_per_mtok: 1.50,
        },
    ),
    (
        "claude-3-opus-latest",
        ModelPricing {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            cache_read_per_mtok: 1.50,
        },
    ),
    // OpenAI GPT-4o（官方公开价：$2.50 / $10.00，cached input $1.25）。
    // 来源：https://openai.com/api/pricing/（2026-10-25 快照）
    (
        "gpt-4o",
        ModelPricing {
            input_per_mtok: 2.50,
            output_per_mtok: 10.0,
            cache_read_per_mtok: 1.25,
        },
    ),
    (
        "gpt-4o-2024-11-20",
        ModelPricing {
            input_per_mtok: 2.50,
            output_per_mtok: 10.0,
            cache_read_per_mtok: 1.25,
        },
    ),
    (
        "gpt-4o-2024-08-06",
        ModelPricing {
            input_per_mtok: 2.50,
            output_per_mtok: 10.0,
            cache_read_per_mtok: 1.25,
        },
    ),
    // OpenAI GPT-4o mini（官方公开价：$0.15 / $0.60，cached input $0.075）。
    // 来源：https://openai.com/api/pricing/（2026-10-25 快照）
    (
        "gpt-4o-mini",
        ModelPricing {
            input_per_mtok: 0.15,
            output_per_mtok: 0.60,
            cache_read_per_mtok: 0.075,
        },
    ),
    (
        "gpt-4o-mini-2024-07-18",
        ModelPricing {
            input_per_mtok: 0.15,
            output_per_mtok: 0.60,
            cache_read_per_mtok: 0.075,
        },
    ),
    // OpenAI GPT-4.1（官方公开价：$2.00 / $8.00，cached input $0.50）。
    // 来源：https://openai.com/api/pricing/（2026-10-25 快照）
    (
        "gpt-4.1",
        ModelPricing {
            input_per_mtok: 2.00,
            output_per_mtok: 8.00,
            cache_read_per_mtok: 0.50,
        },
    ),
    // OpenAI GPT-4.1 mini（官方公开价：$0.40 / $1.60，cached input $0.10）。
    // 来源：https://openai.com/api/pricing/（2026-10-25 快照）
    (
        "gpt-4.1-mini",
        ModelPricing {
            input_per_mtok: 0.40,
            output_per_mtok: 1.60,
            cache_read_per_mtok: 0.10,
        },
    ),
    // OpenAI GPT-4.1 nano（官方公开价：$0.10 / $0.40，cached input $0.025）。
    // 来源：https://openai.com/api/pricing/（2026-10-25 快照）
    (
        "gpt-4.1-nano",
        ModelPricing {
            input_per_mtok: 0.10,
            output_per_mtok: 0.40,
            cache_read_per_mtok: 0.025,
        },
    ),
    // DeepSeek deepseek-chat（V3，官方公开价：cache miss $0.27 / cache hit $0.07 / output $1.10）。
    // 来源：https://api-docs.deepseek.com/quick_start/pricing（2026-10-25 快照）
    (
        "deepseek-chat",
        ModelPricing {
            input_per_mtok: 0.27,
            output_per_mtok: 1.10,
            cache_read_per_mtok: 0.07,
        },
    ),
];

/// 归一化模型名：去空白、转小写、剥离 `provider/` 前缀。
fn normalize_model(model: &str) -> String {
    let trimmed = model.trim().to_ascii_lowercase();
    match trimmed.rsplit_once('/') {
        Some((_provider, tail)) if !tail.is_empty() => tail.to_string(),
        _ => trimmed,
    }
}

/// 查询模型单价；未收录（未知模型）返回 `None`。
///
/// 语义：**只做精确匹配**（大小写不敏感、可带 `provider/` 前缀），
/// 不做模糊/前缀/插值匹配 —— 避免把 `gpt-4o-mini` 误价成 `gpt-4o`。
pub fn pricing_for(model: &str) -> Option<ModelPricing> {
    let normalized = normalize_model(model);
    if normalized.is_empty() {
        return None;
    }
    PRICING_TABLE
        .iter()
        .find(|(known, _)| *known == normalized)
        .map(|(_, pricing)| *pricing)
}

/// 四舍五入到 6 位小数。
fn round_usd(value: f64) -> f64 {
    (value * ROUND_SCALE).round() / ROUND_SCALE
}

/// 单价合法性：三个阶梯都必须是有限且非负的值。
fn is_valid_pricing(pricing: &ModelPricing) -> bool {
    [
        pricing.input_per_mtok,
        pricing.output_per_mtok,
        pricing.cache_read_per_mtok,
    ]
    .iter()
    .all(|value| value.is_finite() && *value >= 0.0)
}

/// 按给定单价计算成本（纯函数，K1 唯一公式实现）。
///
/// 语义：
/// - `cache_read_tokens` 为 `None` 时视为 0；
/// - 未缓存输入 = `input_tokens.saturating_sub(cache_read_tokens)`（缓存读取被剔除，不双重计费）；
/// - 三个输出字段各自四舍五入到 6 位小数；
/// - 单价非法（NaN / ±Inf / 负值）或中间结果非有限 ⇒ `None`。
pub fn calculate_cost_with_pricing(
    pricing: &ModelPricing,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: Option<u64>,
) -> Option<CostEstimate> {
    // 兼容入口：历史语义即 OpenAI 口径（input 含 cache）。
    calculate_cost_with_protocol(
        pricing,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        ProviderCostProtocol::OpenAi,
    )
}

/// 按**协议语义**归一未缓存输入后计价（Wave-6 K1.1 唯一公式实现）。
///
/// 语义：
/// - `Anthropic` ⇒ `uncached = input_tokens`（input 不含 cache，缓存部分另按缓存单价计）；
/// - `OpenAi` / `Unknown` ⇒ `uncached = max(input_tokens − cache_read_tokens, 0)`；
/// - 其余校验（单价非法 / 非有限结果 ⇒ `None`）与 [`calculate_cost_with_pricing`] 完全一致。
pub fn calculate_cost_with_protocol(
    pricing: &ModelPricing,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: Option<u64>,
    protocol: ProviderCostProtocol,
) -> Option<CostEstimate> {
    if !is_valid_pricing(pricing) {
        return None;
    }
    let cache_read = cache_read_tokens.unwrap_or(0);
    let uncached_input = match protocol {
        // Anthropic：input 不含 cache_read，严禁相减（否则 input<cache 时输入费被清零）。
        ProviderCostProtocol::Anthropic => input_tokens,
        // OpenAI / Unknown：prompt_tokens 已含 cached，需剔除（未知协议保守按 OpenAI）。
        ProviderCostProtocol::OpenAi | ProviderCostProtocol::Unknown => {
            input_tokens.saturating_sub(cache_read)
        }
    };

    let input_cost_raw = (uncached_input as f64 * pricing.input_per_mtok
        + cache_read as f64 * pricing.cache_read_per_mtok)
        / TOKENS_PER_MTOK;
    let output_cost_raw = (output_tokens as f64 * pricing.output_per_mtok) / TOKENS_PER_MTOK;
    if !input_cost_raw.is_finite() || !output_cost_raw.is_finite() {
        return None;
    }
    let total_cost_raw = input_cost_raw + output_cost_raw;
    if !total_cost_raw.is_finite() {
        return None;
    }

    let estimate = CostEstimate {
        input_cost_usd: round_usd(input_cost_raw),
        output_cost_usd: round_usd(output_cost_raw),
        total_cost_usd: round_usd(total_cost_raw),
    };
    if !estimate.input_cost_usd.is_finite()
        || !estimate.output_cost_usd.is_finite()
        || !estimate.total_cost_usd.is_finite()
        || estimate.total_cost_usd < 0.0
    {
        return None;
    }
    Some(estimate)
}

/// 按模型名计价：未收录的模型直接返回 `None`（禁止兜底）。
///
/// 调用方：turn 事件装配（K2 `costUsd`）、OTel llm.chat 成本属性（K4）、
/// 会话持久化记录（K2 `TurnTraceRecord.costUsd`）。
pub fn estimate_cost(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: Option<u64>,
) -> Option<CostEstimate> {
    // 兼容入口：历史语义即 OpenAI 口径。
    estimate_cost_with_protocol(
        model,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        ProviderCostProtocol::OpenAi,
    )
}

/// 按模型 + **协议语义**计价（Wave-6 K1.1 接入点唯一入口）。
///
/// 调用方：`turn_flow` 的事件成本（K2）与 `llm.chat` span 成本属性（K4），
/// 必须传 `provider_meta.protocol` 映射出的协议类别，禁止再用无差别减法。
pub fn estimate_cost_with_protocol(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: Option<u64>,
    protocol: ProviderCostProtocol,
) -> Option<CostEstimate> {
    let pricing = pricing_for(model)?;
    calculate_cost_with_protocol(&pricing, input_tokens, output_tokens, cache_read_tokens, protocol)
}

/// 预算超限判定（K3 软告警语义，纯函数，不阻断任何请求）。
///
/// 语义（契约 K3.1「**触及或超过**」⇒ `>=`）：`total_cost_usd >= budget_usd` 即视为超限（等值也告警）；
/// `budget_usd` 为 `None`、非有限值或 `<= 0.0` ⇒ **视为未设置预算**，永不超限；
/// `total_cost_usd` 非有限 ⇒ 视为无法判定（`false`）。
pub fn is_budget_exceeded(total_cost_usd: f64, budget_usd: Option<f64>) -> bool {
    let Some(budget) = budget_usd else {
        return false;
    };
    if !budget.is_finite() || budget <= 0.0 {
        return false;
    }
    if !total_cost_usd.is_finite() {
        return false;
    }
    // K3.1：触及（相等）即算超限。
    total_cost_usd >= budget
}

// ------------------------------------------------------- 进程级预算配置（K3 settings 供给）
//
// 说明：预算只用于「软告警」，不参与任何拦截/限流；宿主（Tauri 设置读写路径）注入，
// 核心只读。`0` 位模式表示「未设置」（与 `budget <= 0` 视为未设置语义一致）。
static BUDGET_USD_BITS: AtomicU64 = AtomicU64::new(0);

/// 设置会话成本预算（`None` / `<= 0` / 非有限 ⇒ 视为未设置）。
///
/// 调用方：`src-tauri` 的 `load_app_settings` / `save_app_settings`（与 OTel 上报开关同构的
/// 设置桥接约定）。
pub fn set_budget_usd(budget: Option<f64>) {
    let bits = match budget {
        Some(value) if value.is_finite() && value > 0.0 => value.to_bits(),
        _ => 0,
    };
    BUDGET_USD_BITS.store(bits, Ordering::SeqCst);
}

/// 读取当前生效的会话成本预算；未设置返回 `None`。
pub fn budget_usd() -> Option<f64> {
    let bits = BUDGET_USD_BITS.load(Ordering::SeqCst);
    if bits == 0 {
        None
    } else {
        Some(f64::from_bits(bits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 价格表自检：单价必须有限且非负、键不得重复（防手滑写坏表）。
    #[test]
    fn pricing_table_is_sane_and_unique() {
        for (model, pricing) in PRICING_TABLE {
            assert!(is_valid_pricing(pricing), "价格表条目非法：{model}");
            assert!(
                pricing.cache_read_per_mtok <= pricing.input_per_mtok,
                "缓存读取价不应高于未缓存输入价：{model}"
            );
        }
        let mut seen = std::collections::HashSet::new();
        for (model, _) in PRICING_TABLE {
            assert!(seen.insert(*model), "价格表重复键：{model}");
        }
    }

    /// 未收录的仓库默认模型（虚构型号）必须计价为 None —— 宁可「—」也不编造价格。
    #[test]
    fn unlisted_repo_default_models_have_no_price() {
        for model in [
            "gpt-5.5",
            "gpt-5-alpha",
            "gemini-3.8-flash",
            "deepseek-v4-flash",
            "my-custom-fine-tuned-model-xyz",
        ] {
            assert!(
                pricing_for(model).is_none(),
                "{model} 不应出现在价格表（无公开可核价依据）"
            );
            assert!(estimate_cost(model, 1000, 500, None).is_none());
        }
    }

    /// 精确匹配：不得把 `gpt-4o-mini` 误价成 `gpt-4o`；允许 `provider/` 前缀与大小写差异。
    #[test]
    fn pricing_lookup_is_exact_but_tolerates_prefix_and_case() {
        let mini = pricing_for("openai/GPT-4o-mini").expect("gpt-4o-mini 必须命中");
        assert_eq!(mini.input_per_mtok, 0.15);
        let full = pricing_for("gpt-4o").expect("gpt-4o 必须命中");
        assert_eq!(full.input_per_mtok, 2.50);
        assert!(pricing_for("gpt-4o-extra-unknown").is_none());
    }

    /// 零 token 边界：命中价格表时返回有限 0.0（契约 K1 边界 3）。
    #[test]
    fn zero_tokens_return_finite_zero() {
        let estimate = estimate_cost("gpt-4o", 0, 0, None).expect("零 token 应返回 Some");
        assert_eq!(estimate.total_cost_usd, 0.0);
    }

    /// FIX-1：契约 K3.1「触及或超过」⇒ 等值即超限（`>=`），且未设置预算永不超限。
    #[test]
    fn budget_threshold_is_inclusive() {
        assert!(is_budget_exceeded(0.50, Some(0.50)), "等值必须判定超限（触及=超限）");
        assert!(is_budget_exceeded(0.500001, Some(0.50)));
        assert!(!is_budget_exceeded(0.499999, Some(0.50)));
        assert!(!is_budget_exceeded(0.50, None), "未设置预算永不超限");
        assert!(!is_budget_exceeded(0.50, Some(0.0)), "0 视为未设置");
        assert!(!is_budget_exceeded(0.50, Some(-1.0)), "负数视为未设置");
        assert!(!is_budget_exceeded(f64::NAN, Some(0.5)), "非有限成本不判定超限");
    }

    /// K1.1：协议标签映射（Anthropic 族 / OpenAI 族 / 未知保守回退）。
    #[test]
    fn protocol_label_mapping_is_explicit_and_conservative() {
        for label in ["anthropic", "anthropic-messages", "bedrock", "Claude"] {
            assert_eq!(
                ProviderCostProtocol::from_label(label),
                ProviderCostProtocol::Anthropic,
                "{label} 必须归为 Anthropic 族"
            );
        }
        for label in ["openai", "openai-completions", "openai-responses", "deepseek", "azure", "ollama"] {
            assert_eq!(
                ProviderCostProtocol::from_label(label),
                ProviderCostProtocol::OpenAi,
                "{label} 必须归为 OpenAI 族"
            );
        }
        for label in ["", "   ", "mystery-protocol"] {
            assert_eq!(
                ProviderCostProtocol::from_label(label),
                ProviderCostProtocol::Unknown,
                "未知标签必须保守归为 Unknown（按 OpenAI 语义计）"
            );
        }
    }

    /// K1.1：Anthropic 语义下 input<cache 时不得把基础输入费清零（历史缺陷）。
    #[test]
    fn anthropic_semantics_does_not_zero_uncached_input() {
        let pricing = ModelPricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.3,
        };
        let anthropic = calculate_cost_with_protocol(
            &pricing,
            100,
            0,
            Some(5000),
            ProviderCostProtocol::Anthropic,
        )
        .expect("Anthropic 计价成功");
        assert_eq!(anthropic.input_cost_usd, 0.0018);
        let openai = calculate_cost_with_protocol(
            &pricing,
            100,
            0,
            Some(5000),
            ProviderCostProtocol::OpenAi,
        )
        .expect("OpenAI 计价成功");
        assert_eq!(openai.input_cost_usd, 0.0015);
    }

    /// 预算桥接：非正/非有限值一律视为未设置；设置后可读回。
    #[test]
    fn budget_bridge_treats_non_positive_as_unset() {
        set_budget_usd(Some(-1.0));
        assert_eq!(budget_usd(), None);
        set_budget_usd(Some(f64::NAN));
        assert_eq!(budget_usd(), None);
        set_budget_usd(Some(0.0));
        assert_eq!(budget_usd(), None);
        set_budget_usd(Some(1.25));
        assert_eq!(budget_usd(), Some(1.25));
        set_budget_usd(None);
        assert_eq!(budget_usd(), None);
    }
}
