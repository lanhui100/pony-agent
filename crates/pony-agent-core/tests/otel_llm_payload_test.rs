//! P0-1 红相验收与对抗测试：`llm.chat` 与 `agent.turn` span 携带脱敏 + 截断的 Prompt 与 Completion 预览。
//!
//! 契约真源：`.dev-team/contract-matrix-p0-1.md` 与 `.dev-team/nfr-baseline-p0-1.json`。
//!
//! 语义双轨契约：
//! - OTel 轨：`gen_ai.prompt` 与 `gen_ai.completion`
//! - Langfuse 轨：`langfuse.observation.input` 与 `langfuse.observation.output`
//!
//! 硬性验收要求：
//! 1. L2-T 功能契约：
//!    - 有 prompt / completion 预览 ⇒ 写出双轨键（`gen_ai.prompt` / `gen_ai.completion` 及 Langfuse 对偶键）；
//!    - 敏感信息（Bearer / sk- / 家目录）⇒ 前置强制脱敏（复用 `ponysentry::sanitize`）；
//!    - 超长预览 ⇒ UTF-8 安全截断且带 `…[truncated]` 标记；
//!    - 开关控制 ⇒ 环境变量 `PONY_AGENT_CAPTURE_LLM_PAYLOADS=0` (或 false/off/no) 时完全不输出上述 payload 属性键。
//! 2. L2-AT 破坏性/对抗边界：
//!    - 超长文本（>4KB、>8KB）以及多字节 UTF-8 中文字符边界注入截断不 panic 且末尾字符不破坏。

use std::sync::{Mutex, MutexGuard};

use pony_agent_core::agent::otel::{
    llm_span_attributes, turn_span_attributes, AttrValue, LlmSpanInput, TurnSpanInput,
};

/// 开关控制环境变量（契约定义：PONY_AGENT_CAPTURE_LLM_PAYLOADS，默认开启）
const CAPTURE_ENV: &str = "PONY_AGENT_CAPTURE_LLM_PAYLOADS";

/// OTel GenAI 语义轨常量键（契约字面量）
const KEY_GEN_AI_PROMPT: &str = "gen_ai.prompt";
const KEY_GEN_AI_COMPLETION: &str = "gen_ai.completion";

/// Langfuse 语义轨常量键
const KEY_LANGFUSE_INPUT: &str = "langfuse.observation.input";
const KEY_LANGFUSE_OUTPUT: &str = "langfuse.observation.output";

/// 基线单字段预览上限（4KB = 4096 字节）
const PREVIEW_LIMIT_BYTES_4K: usize = 4096;

/// 并发安全串行锁
static CAPTURE_LOCK: Mutex<()> = Mutex::new(());

fn capture_lock() -> MutexGuard<'static, ()> {
    CAPTURE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 确保清理环境变量为默认开启态
fn ensure_capture_default_on() {
    std::env::remove_var(CAPTURE_ENV);
}

/// RAII 环境变量守卫，在测试退出或 panic 时自动清理
struct EnvVarGuard {
    key: &'static str,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        std::env::set_var(key, value);
        Self { key }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        std::env::remove_var(self.key);
    }
}

/// 从 attributes 列表中按键读取字符串值
fn attr_str<'a>(attributes: &'a [(String, AttrValue)], key: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, value)| match value {
            AttrValue::Str(text) => Some(text.as_str()),
            _ => None,
        })
}

// =========================================================================
// 1. L2-T 功能契约测试
// =========================================================================

#[test]
fn llm_chat_span_payload_preview_sanitized_and_truncated() {
    let _gate = capture_lock();
    ensure_capture_default_on();

    // Arrange: 准备包含敏感凭据、系统路径以及长文本的请求与响应
    const SENSITIVE_TOKEN: &str = "sk-test-TOPSECRET-9876543210";
    const BARE_TOKEN: &str = "sk-mock-ant-api03-token-1234567890abcdef";
    let sensitive_prompt = format!(
        "User asked: please check file /home/dm/pony-agent/config.json with Authorization: Bearer {SENSITIVE_TOKEN} and secret {BARE_TOKEN}"
    );

    let long_completion = format!(
        "Assistant replied: OK, here is the answer: {}",
        "A".repeat(5000)
    );

    // 构造带 prompt 与 completion 预览的 LlmSpanInput
    // 待实现契约：LlmSpanInput 需增加 prompt_preview 与 completion_preview 字段
    let input = LlmSpanInput {
        provider_kind: "openai".to_string(),
        model: Some("gpt-4o".to_string()),
        input_tokens: Some(100),
        output_tokens: Some(200),
        operation: "chat".to_string(),
        prompt_preview: Some(sensitive_prompt.clone()),
        completion_preview: Some(long_completion.clone()),
    };

    // Act
    let attributes = llm_span_attributes(&input);

    // Assert 1: 双轨属性键必须存在
    let prompt_val = attr_str(&attributes, KEY_GEN_AI_PROMPT)
        .or_else(|| attr_str(&attributes, KEY_LANGFUSE_INPUT))
        .expect("P0-1 契约：llm.chat span 必须包含 gen_ai.prompt 或 langfuse.observation.input");

    let completion_val = attr_str(&attributes, KEY_GEN_AI_COMPLETION)
        .or_else(|| attr_str(&attributes, KEY_LANGFUSE_OUTPUT))
        .expect("P0-1 契约：llm.chat span 必须包含 gen_ai.completion 或 langfuse.observation.output");

    // Assert 2: 敏感 Token 与本地用户路径必须已被脱敏
    assert!(
        !prompt_val.contains(SENSITIVE_TOKEN),
        "P0-1 契约：prompt 中不得包含原始敏感 Token"
    );
    assert!(
        !prompt_val.contains("/home/dm"),
        "P0-1 契约：prompt 中本地路径必须被脱敏为 [USER_HOME]"
    );
    assert!(
        prompt_val.contains("[REDACTED_SECRET]") || prompt_val.contains("[USER_HOME]"),
        "P0-1 契约：prompt 必须包含脱敏替换标记"
    );

    // Assert 3: 超过 4KB 的 completion 必须截断且长度 ≤ 4096 字节，并带截断标记
    assert!(
        completion_val.len() <= PREVIEW_LIMIT_BYTES_4K,
        "P0-1 契约：completion 截断后字节数必须 ≤ {}，实际 {}",
        PREVIEW_LIMIT_BYTES_4K,
        completion_val.len()
    );
    assert!(
        completion_val.contains("…[truncated]") || completion_val.contains("truncated"),
        "P0-1 契约：超长 completion 截断后必须包含截断标记"
    );
}

#[test]
fn agent_turn_span_payload_preview_contract() {
    let _gate = capture_lock();
    ensure_capture_default_on();

    // Arrange: 验证 agent.turn root span 也遵循双轨 Payload 契约
    let input = TurnSpanInput {
        session_id: "sess-1".to_string(),
        user_id: Some("user-1".to_string()),
        turn_id: "turn-100".to_string(),
        sequence: 1,
        phase: "completed".to_string(),
        failed_reason: None,
        prompt_preview: Some("Turn user prompt with /home/dm/path".to_string()),
        response_preview: Some("Turn final response".to_string()),
    };

    // Act
    let attributes = turn_span_attributes(&input);

    // Assert: agent.turn span 包含 Prompt 与 Response 属性且脱敏
    let prompt_val = attr_str(&attributes, KEY_GEN_AI_PROMPT)
        .or_else(|| attr_str(&attributes, KEY_LANGFUSE_INPUT))
        .expect("P0-1 契约：agent.turn span 必须包含 gen_ai.prompt 或 langfuse.observation.input");

    assert!(
        !prompt_val.contains("/home/dm"),
        "P0-1 契约：agent.turn prompt 中路径必须被脱敏"
    );
    assert_eq!(
        attr_str(&attributes, KEY_GEN_AI_COMPLETION).or_else(|| attr_str(&attributes, KEY_LANGFUSE_OUTPUT)),
        Some("Turn final response")
    );
}

#[test]
fn llm_payload_capture_switch_opt_out() {
    let _gate = capture_lock();
    // Arrange: 显式关闭 Capture 开关
    let _env = EnvVarGuard::set(CAPTURE_ENV, "0");

    let input = LlmSpanInput {
        provider_kind: "openai".to_string(),
        model: Some("gpt-4o".to_string()),
        input_tokens: Some(10),
        output_tokens: Some(20),
        operation: "chat".to_string(),
        prompt_preview: Some("Hello".to_string()),
        completion_preview: Some("Hi there".to_string()),
    };

    // Act
    let attributes = llm_span_attributes(&input);

    // Assert: 开关关闭时，不得输出任何 prompt 或 completion 属性
    for key in [
        KEY_GEN_AI_PROMPT,
        KEY_GEN_AI_COMPLETION,
        KEY_LANGFUSE_INPUT,
        KEY_LANGFUSE_OUTPUT,
    ] {
        assert!(
            attributes.iter().all(|(name, _)| name != key),
            "P0-1 契约：{CAPTURE_ENV}=0 时不得写出 {key}；实际 attributes={attributes:?}"
        );
    }

    // 核心业务属性仍需保留
    assert!(
        attributes.iter().any(|(name, _)| name == "gen_ai.system"),
        "开关关闭不得影响基础 span 属性"
    );
}

// =========================================================================
// 2. L2-AT 对抗注入与边界测试 (Adversarial & Robustness)
// =========================================================================

#[test]
fn adversarial_oversized_multi_byte_utf8_truncation_no_panic() {
    let _gate = capture_lock();
    ensure_capture_default_on();

    // Arrange: 构造超长中文（每个中文字符占用 3 字节 UTF-8），使 4096 字节预算处恰好处于三字节字符中间
    // "测试中文字符串" 重复 1000 次（约 21000 字节，远超 4KB / 8KB 上限）
    let chinese_chunk = "测试中文多字节边界，严禁产生非法UTF8！";
    let huge_chinese_payload = chinese_chunk.repeat(300);

    let input = LlmSpanInput {
        provider_kind: "anthropic".to_string(),
        model: Some("claude-3-5-sonnet".to_string()),
        input_tokens: Some(500),
        output_tokens: Some(1000),
        operation: "chat".to_string(),
        prompt_preview: Some(huge_chinese_payload.clone()),
        completion_preview: Some(huge_chinese_payload.clone()),
    };

    // Act: 必须保证截断函数内部安全回退到 is_char_boundary，绝不切断中文字符产生 panic
    let attributes = llm_span_attributes(&input);

    for key in [KEY_GEN_AI_PROMPT, KEY_GEN_AI_COMPLETION] {
        if let Some(val) = attr_str(&attributes, key) {
            // Assert 1: 必须符合有效 UTF-8（val 本身即是 &str，若切分错误内部在转换为 String 时早已 panic）
            assert!(
                val.len() <= PREVIEW_LIMIT_BYTES_4K,
                "对抗断言：截断后字节长度必须 ≤ 4096"
            );
            assert!(
                val.contains("…[truncated]") || val.contains("truncated"),
                "对抗断言：必须包含截断标记"
            );
        }
    }
}
