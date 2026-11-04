use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

static RE_UNIX_HOME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/home/[^/\s]+").unwrap());
static RE_MAC_HOME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/Users/[^/\s]+").unwrap());
static RE_WIN_HOME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[A-Za-z]:\\Users\\[^\s\\]+").unwrap());

// ── 裸 token 形态（P0-1 安全前置）───────────────────────────────────────────────
//
// 背景：既有规则只覆盖「键值 / Bearer / Basic」形态，**裸 token**（如 `sk-live-…` 直接出现在
// 命令行、JSON 之外的文本、URL 片段里）会原样留在 span 属性中。P0-1 默认开启工具 span 的
// 入参/结果预览后，这就是真实的凭据外泄面，故在此补齐业界常见前缀形态。
//
// 取向：**宁可多脱敏**。这些前缀（`sk-` / `ghp_` / `xox?-` / `AKIA` / `AIza`）在正常业务文本中
// 极少出现，误伤代价远小于漏报代价。
/// OpenAI / Anthropic 形态（含 `sk-ant-`、`sk-proj-` 等变体）：`sk-` 后 ≥16 个 token 字符。
static RE_SK_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bsk-[A-Za-z0-9_-]{16,}").unwrap());
/// GitHub 形态：`ghp_` / `gho_` / `ghs_` / `ghu_` + ≥20 位。
static RE_GITHUB_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bgh[pous]_[A-Za-z0-9]{20,}").unwrap());
/// Slack 形态：`xox[abposr]-` + ≥10 位。
static RE_SLACK_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bxox[abposr]-[A-Za-z0-9-]{10,}").unwrap());
/// AWS Access Key ID 形态。
static RE_AWS_ACCESS_KEY_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap());
/// Google API key 形态。
static RE_GOOGLE_API_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bAIza[0-9A-Za-z_-]{35,}").unwrap());

static RE_BEARER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)bearer\s+[a-zA-Z0-9_\.\-]{10,}").unwrap());
static RE_BASIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)basic\s+[a-zA-Z0-9+/=]{10,}").unwrap());

/// 敏感键名判定（Wave-4.1 L3-A #6）：由「整名精确匹配」改为**子串/后缀匹配**，
/// 以覆盖 `otelToken` / `client_secret` / `dbPassword` / `aws_credentials` 这类真实命名。
///
/// 安全取向：宁可多脱敏也不漏。计量类键（`input_tokens` / `tokenCount` 等）
/// 由 [`is_metric_or_token_count_key`] 在匹配前白名单直通，避免误伤诊断数据。
static SENSITIVE_KEY_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(token|secret|password|passwd|credential|api_?key|apikey|authorization|cookie|private_?key)",
    )
    .unwrap()
});

static RE_KEY_VALUE_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)"(token|password|passwd|secret|api_key|apikey|access_token|refresh_token|authorization|credential)"\s*:\s*"[^"]+""#).unwrap()
});

/// 计量/用量键**精确白名单**（Wave-4.1 H2）。
///
/// 安全取向：只放行本系统自己产出的计量键名，且**值必须是数字**；
/// 任何非枚举的 `*token*` 键（如 `access_tokens` / `refresh_tokens` / `id_token`）
/// 一律按凭据脱敏 —— 用后缀 `ends_with("tokens")` 判定会把真凭据误当计量值放行。
const TOKEN_METRIC_KEYS: &[&str] = &[
    "input_tokens",
    "output_tokens",
    "total_tokens",
    "prompt_tokens",
    "completion_tokens",
    "reasoning_tokens",
    "cached_input_tokens",
    "cache_hit_input_tokens",
    "cache_miss_input_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
];

#[inline]
fn is_metric_or_token_count_key(key: &str, value: &Value) -> bool {
    // 值必须是数字：即使键名命中白名单，字符串/对象/数组形态一律走脱敏
    // （防「键名巧合 + 凭据值」组合漏网）。
    value.is_number() && TOKEN_METRIC_KEYS.iter().any(|known| key.eq_ignore_ascii_case(known))
}

pub fn sanitize(input: &str) -> String {
    let mut s = input.to_string();

    // 1. Path redaction
    s = RE_UNIX_HOME.replace_all(&s, "[USER_HOME]").to_string();
    s = RE_MAC_HOME.replace_all(&s, "[USER_HOME]").to_string();
    s = RE_WIN_HOME.replace_all(&s, "[USER_HOME]").to_string();

    // 2. Bearer / Basic tokens
    s = RE_BEARER.replace_all(&s, "Bearer [REDACTED_SECRET]").to_string();
    s = RE_BASIC.replace_all(&s, "Basic [REDACTED_SECRET]").to_string();

    // 2b. 裸 token 形态（P0-1）：无键名/无 `Bearer` 前缀也要脱敏。
    // 顺序放在 Bearer/Basic 之后：带前缀的形态已整体替换，此处只兜裸 token。
    s = RE_SK_TOKEN.replace_all(&s, "[REDACTED_SECRET]").to_string();
    s = RE_GITHUB_TOKEN.replace_all(&s, "[REDACTED_SECRET]").to_string();
    s = RE_SLACK_TOKEN.replace_all(&s, "[REDACTED_SECRET]").to_string();
    s = RE_AWS_ACCESS_KEY_ID.replace_all(&s, "[REDACTED_SECRET]").to_string();
    s = RE_GOOGLE_API_KEY.replace_all(&s, "[REDACTED_SECRET]").to_string();

    // 3. Key-Value secrets in JSON strings like {"apiKey": "xxx"}
    s = RE_KEY_VALUE_SECRET
        .replace_all(&s, |caps: &regex::Captures| {
            let key = &caps[1];
            format!(r#""{key}": "[REDACTED_SECRET]""#)
        })
        .to_string();

    s
}

pub fn sanitize_json(value: &Value) -> Value {
    sanitize_json_depth(value, 0)
}

fn sanitize_json_depth(value: &Value, depth: usize) -> Value {
    if depth > 32 {
        return Value::String("[MAX_DEPTH_EXCEEDED]".to_string());
    }

    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                if is_metric_or_token_count_key(k, v) {
                    // H2：仅「精确白名单键 + 数字值」直通，其余一律走脱敏分支。
                    out.insert(k.clone(), sanitize_json_depth(v, depth + 1));
                } else if SENSITIVE_KEY_PATTERN.is_match(k) {
                    out.insert(k.clone(), Value::String("[REDACTED_SECRET]".to_string()));
                } else {
                    out.insert(k.clone(), sanitize_json_depth(v, depth + 1));
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                out.push(sanitize_json_depth(item, depth + 1));
            }
            Value::Array(out)
        }
        Value::String(s) => Value::String(sanitize(s)),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// H2：计量键白名单必须是**精确键名 + 数字值**，不得放行 `access_tokens` 这类真凭据。
    #[test]
    fn h2_token_metric_whitelist_is_exact_and_numeric_only() {
        let sanitized = sanitize_json(&json!({
            "access_tokens": "secret123",
            "refresh_tokens": "refresh-secret",
            "id_token": "eyJhbGciOi.payload.sig",
            "input_tokens": 42,
            "output_tokens": 7,
            "total_tokens": 49,
            "cache_hit_input_tokens": 12,
            // 键名命中白名单但值是字符串 ⇒ 仍必须脱敏
            "reasoning_tokens": "not-a-number-Bearer abcdefghij"
        }));

        assert_eq!(sanitized["access_tokens"], "[REDACTED_SECRET]");
        assert_eq!(sanitized["refresh_tokens"], "[REDACTED_SECRET]");
        assert_eq!(sanitized["id_token"], "[REDACTED_SECRET]");
        assert_eq!(sanitized["input_tokens"], 42);
        assert_eq!(sanitized["output_tokens"], 7);
        assert_eq!(sanitized["total_tokens"], 49);
        assert_eq!(sanitized["cache_hit_input_tokens"], 12);
        assert_eq!(
            sanitized["reasoning_tokens"], "[REDACTED_SECRET]",
            "H2：白名单键值非数字时必须走脱敏"
        );

        let rendered = serde_json::to_string(&sanitized).expect("serialize");
        assert!(!rendered.contains("secret123"), "凭据泄漏：{rendered}");
        assert!(!rendered.contains("abcdefghij"), "凭据泄漏：{rendered}");
    }

    /// P0-1 安全前置：**裸 token**（无键名 / 无 `Bearer` 前缀）必须被识别并整体替换。
    ///
    /// 覆盖 OpenAI/Anthropic、GitHub、Slack、AWS、Google 五类常见前缀形态；
    /// 同时校验**不误伤**普通短词（否则会污染 trace 可读性）。
    #[test]
    fn bare_tokens_are_redacted_by_prefix_shape() {
        let cases = [
            (
                "sk",
                "curl -H 'x' sk-test-TOPSECRET-9876543210",
                "sk-test-TOPSECRET-9876543210",
            ),
            (
                "sk-ant",
                "key=sk-ant-api03-fakeabcdefghijklmnopqrstuvwxyz",
                "sk-ant-api03-fakeabcdefghijklmnopqrstuvwxyz",
            ),
            (
                "sk-proj",
                "sk-proj-fakeAbCdEfGhIjKlMnOpQrStUvWx",
                "sk-proj-fakeAbCdEfGhIjKlMnOpQrStUvWx",
            ),
            (
                "github",
                "token ghp_testABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
                "ghp_testABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
            ),
            (
                "github-oauth",
                "gho_testABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
                "gho_testABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
            ),
            (
                "github-server",
                "ghs_testABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
                "ghs_testABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
            ),
            (
                "github-user",
                "ghu_testABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
                "ghu_testABCDEFGHIJKLMNOPQRSTUVWXYZ012345",
            ),
            (
                "slack",
                "xoxb-test-1234567890-abcdefghij",
                "xoxb-test-1234567890-abcdefghij",
            ),
            (
                "slack-app",
                "xoxa-test-1234567890-abcdefghij",
                "xoxa-test-1234567890-abcdefghij",
            ),
            (
                "aws",
                "AKIAIOSFODNN7EXAMPLE",
                "AKIAIOSFODNN7EXAMPLE",
            ),
            (
                "google",
                "AIzaSyA1234567890abcdefghijklmnopqrstuv",
                "AIzaSyA1234567890abcdefghijklmnopqrstuv",
            ),
        ];

        for (label, input, secret) in cases {
            let sanitized = sanitize(input);
            assert!(
                !sanitized.contains(secret),
                "P0-1：{label} 形态的裸 token 必须脱敏，实际={sanitized}"
            );
            assert!(
                sanitized.contains("[REDACTED_SECRET]"),
                "P0-1：{label} 形态必须替换为 [REDACTED_SECRET]，实际={sanitized}"
            );
        }

        // 不误伤：普通短词/普通文件名不得被替换。
        for benign in ["sk-short", "task-runner", "disk-usage-report", "ghost_writer"] {
            let sanitized = sanitize(benign);
            assert_eq!(
                sanitized, benign,
                "P0-1：普通文本 `{benign}` 不得被裸 token 规则误伤"
            );
        }
    }

    /// H2 回归：非枚举的 `*token*` 键一律脱敏（含大小写变体与后缀形态）。
    #[test]
    fn h2_any_unlisted_token_key_is_redacted() {
        let sanitized = sanitize_json(&json!({
            "session_token_count": "Bearer abcdefghij",
            "TokenUsage": {"a": 1},
            "max_tokens": "should-be-redacted"
        }));
        assert_eq!(sanitized["session_token_count"], "[REDACTED_SECRET]");
        assert_eq!(sanitized["TokenUsage"], "[REDACTED_SECRET]");
        assert_eq!(sanitized["max_tokens"], "[REDACTED_SECRET]");
    }
}
