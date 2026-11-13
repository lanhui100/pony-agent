// usage: usage 原始字段解析与归一（OpenAI 家族 usage → TokenUsage）。
// 由 provider/mod.rs 拆分而来，行为与结构保持逐字一致（语义零变更）。
use serde_json::Value;

use super::{provider_log, TokenUsage, USAGE_ORIGIN_PROVIDER};

pub(crate) fn extract_openai_usage(payload: &Value) -> Option<TokenUsage> {
    let usage = payload.get("usage")?;
    provider_log(format!("usage:openai raw={}", usage));
    let cache_hit = usage
        .get("prompt_cache_hit_tokens")
        .and_then(Value::as_u64)
        .map(|value| (value, "usage.prompt_cache_hit_tokens"))
        .or_else(|| {
            usage
                .get("input_tokens_details")
                .and_then(|value| value.get("cached_tokens"))
                .and_then(Value::as_u64)
                .map(|value| (value, "usage.input_tokens_details.cached_tokens"))
        })
        .or_else(|| {
            usage
                .get("prompt_tokens_details")
                .and_then(|value| value.get("cached_tokens"))
                .and_then(Value::as_u64)
                .map(|value| (value, "usage.prompt_tokens_details.cached_tokens"))
        });
    let cache_hit_tokens = cache_hit.map(|(value, _source)| value);
    // DeepSeek 原生字段：`prompt_tokens` 缺失时用 `prompt_cache_hit_tokens + prompt_cache_miss_tokens`
    // 还原 prompt 总量（两字段俱在才算，只有 hit 不足以代表总量）。
    // 依据：ADR `.agents/notes/implemented/bug-fix/2026-11-12-metrics-percent-honesty.md`。
    let cache_miss_tokens = usage
        .get("prompt_cache_miss_tokens")
        .and_then(Value::as_u64);
    let input_tokens = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            let hit = cache_hit_tokens?;
            let miss = cache_miss_tokens?;
            Some(hit.saturating_add(miss))
        });
    Some(normalize_token_usage(TokenUsage {
        input_tokens,
        cache_hit_input_tokens: cache_hit_tokens,
        cache_hit_source: cache_hit.map(|(_value, source)| source.to_string()),
        reasoning_tokens: usage
            .get("completion_tokens_details")
            .and_then(|value| value.get("reasoning_tokens"))
            .and_then(Value::as_u64)
            .or_else(|| {
                usage
                    .get("output_tokens_details")
                    .and_then(|value| value.get("reasoning_tokens"))
                    .and_then(Value::as_u64)
            }),
        output_tokens: usage.get("completion_tokens").and_then(Value::as_u64),
        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
        cache_creation_input_tokens: None,
        // wave-54 C3：上游 usage 真值。
        origin: Some(USAGE_ORIGIN_PROVIDER),
    }))
}

pub(crate) fn normalize_token_usage(usage: TokenUsage) -> TokenUsage {
    let total_tokens =
        usage
            .total_tokens
            .or_else(|| match (usage.input_tokens, usage.output_tokens) {
                (Some(input_tokens), Some(output_tokens)) => Some(input_tokens + output_tokens),
                _ => None,
            });

    TokenUsage {
        input_tokens: usage.input_tokens,
        cache_hit_input_tokens: usage.cache_hit_input_tokens,
        cache_hit_source: usage.cache_hit_source,
        reasoning_tokens: usage.reasoning_tokens,
        output_tokens: usage.output_tokens,
        total_tokens,
        cache_creation_input_tokens: usage.cache_creation_input_tokens,
        origin: usage.origin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_usage_falls_back_to_cache_hit_plus_miss_when_prompt_tokens_absent() {
        // DeepSeek 原生形态：部分兼容网关只给 hit/miss 两桶，不给 prompt_tokens。
        let payload = json!({
            "usage": {
                "prompt_cache_hit_tokens": 30,
                "prompt_cache_miss_tokens": 70,
                "completion_tokens": 20,
                "total_tokens": 120
            }
        });

        let usage = extract_openai_usage(&payload).expect("usage should be extracted");
        assert_eq!(usage.input_tokens, Some(100), "prompt 总量 = hit + miss");
        assert_eq!(usage.cache_hit_input_tokens, Some(30));

        // 缺 miss ⇒ 不得半猜出一个更小的「总量」。
        let partial = json!({
            "usage": {
                "prompt_cache_hit_tokens": 30,
                "completion_tokens": 20
            }
        });
        let partial_usage = extract_openai_usage(&partial).expect("usage should be extracted");
        assert_eq!(partial_usage.input_tokens, None);
        assert_eq!(partial_usage.cache_hit_input_tokens, Some(30));
    }
}
