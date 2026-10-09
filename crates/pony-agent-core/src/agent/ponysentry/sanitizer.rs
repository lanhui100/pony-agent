use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

static RE_UNIX_HOME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/home/[^/\s]+").unwrap());
static RE_MAC_HOME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/Users/[^/\s]+").unwrap());
static RE_WIN_HOME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[A-Za-z]:\\Users\\[^\s\\]+").unwrap());

static RE_BEARER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)bearer\s+[a-zA-Z0-9_\.\-]{10,}").unwrap());
static RE_BASIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)basic\s+[a-zA-Z0-9+/=]{10,}").unwrap());

static SENSITIVE_KEY_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(token|password|passwd|secret|api_key|apikey|access_token|refresh_token|authorization|cookie|private_key|credential)$").unwrap()
});

static RE_KEY_VALUE_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)"(token|password|passwd|secret|api_key|apikey|access_token|refresh_token|authorization|credential)"\s*:\s*"[^"]+""#).unwrap()
});

#[inline]
fn is_metric_or_token_count_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.ends_with("tokens") || lower.contains("token_count") || lower.contains("tokencount")
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
                if is_metric_or_token_count_key(k) {
                    // 数值型与计量 tokens 字段白名单直通，严禁脱敏
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
