//! P0-1 红相验收：工具 span 携带**脱敏 + 截断**的入参/结果预览（trace 可复盘的最小切片）。
//!
//! 契约真源：trace 报告 P0-1（trace 无内容 ⇒ 无法复盘）。
//! 目标：`ToolSpanInput` 增 `args_preview` / `result_preview`，由纯函数
//! [`tool_span_attributes`] 映射到 Langfuse 观察键：
//! - `langfuse.observation.input` ← args 预览
//! - `langfuse.observation.output` ← result 预览
//!
//! 硬要求：
//! 1. 有预览 ⇒ 写键，值为（脱敏 + 截断后的）预览；
//! 2. 超长 ⇒ 上限 2KB 且带截断标记（禁止静默超限）；
//! 3. 含敏感串 ⇒ 复用 `ponysentry::sanitize` 后不得出现原串；
//! 4. `None` ⇒ **不写键**（禁止空串占位）；
//! 5. 开关 `PONY_OTEL_CAPTURE_TOOL_PAYLOADS=0` ⇒ 两键都不出现（默认开启）。
//!
//! 本文件只测**纯函数**：不起网络、不装导出器、不读全局 provider。
//!
//! ## env 串行约束
//! `PONY_OTEL_CAPTURE_TOOL_PAYLOADS` 是进程级全局；`cargo test` 同二进制内多线程并发跑用例，
//! 故本文件**所有**用例持 `CAPTURE_LOCK` 串行，env 一律经 `EnvVarGuard`（Drop 复位，panic 安全）。

use std::sync::{Mutex, MutexGuard};

use pony_agent_core::agent::otel::{tool_span_attributes, AttrValue, ToolSpanInput};
use pony_agent_core::agent::ponysentry::sanitize;

/// capture 开关 env（默认开启；`0` 关闭）。
const CAPTURE_ENV: &str = "PONY_OTEL_CAPTURE_TOOL_PAYLOADS";

/// Langfuse 观察键（契约字面量，不从实现常量取，避免「测试跟着实现改」）。
const KEY_INPUT: &str = "langfuse.observation.input";
const KEY_OUTPUT: &str = "langfuse.observation.output";

/// 预览上限（契约 2KB）。
const PREVIEW_LIMIT_BYTES: usize = 2 * 1024;

static CAPTURE_LOCK: Mutex<()> = Mutex::new(());

fn capture_lock() -> MutexGuard<'static, ()> {
    CAPTURE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 确保「默认开启」态：清掉可能被前序用例/宿主设置过的 env。
fn ensure_capture_default_on() {
    std::env::remove_var(CAPTURE_ENV);
}

/// env 守卫：`set_var` + Drop 复位（panic 安全）。
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

/// 构造带预览的工具 span 输入（`args_preview` / `result_preview` 是 P0-1 新增字段）。
fn tool_input(args: Option<&str>, result: Option<&str>) -> ToolSpanInput {
    ToolSpanInput {
        tool_name: "workspace_run_command".to_string(),
        status: "ok".to_string(),
        duration_ms: Some(300),
        error: None,
        args_preview: args.map(str::to_string),
        result_preview: result.map(str::to_string),
    }
}

/// 取字符串属性（类型不符或缺失均返回 `None`）。
fn attr_str<'a>(attributes: &'a [(String, AttrValue)], key: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, value)| match value {
            AttrValue::Str(text) => Some(text.as_str()),
            _ => None,
        })
}

// ── 契约 1：有预览 ⇒ 写键且值等于预览 ──────────────────────────────────────

#[test]
fn previews_are_mapped_to_langfuse_observation_keys() {
    let _gate = capture_lock();
    ensure_capture_default_on();

    // Arrange
    let args = r#"{"command":"echo hi","cwd":"/repo"}"#;
    let result = r#"{"ok":true,"exitCode":0,"stdout":"hi"}"#;
    let input = tool_input(Some(args), Some(result));

    // Act
    let attributes = tool_span_attributes(&input);

    // Assert
    assert_eq!(
        attr_str(&attributes, KEY_INPUT),
        Some(args),
        "P0-1：{KEY_INPUT} 必须等于入参预览；实际属性={attributes:?}"
    );
    assert_eq!(
        attr_str(&attributes, KEY_OUTPUT),
        Some(result),
        "P0-1：{KEY_OUTPUT} 必须等于结果预览；实际属性={attributes:?}"
    );
}

// ── 契约 2：超长 ⇒ ≤2KB 且带截断标记 ───────────────────────────────────────

#[test]
fn oversized_previews_are_truncated_with_marker() {
    let _gate = capture_lock();
    ensure_capture_default_on();

    // Arrange：10KB 纯 ASCII（字节数 == 字符数，避免多字节干扰上限判定）。
    let huge_args = "x".repeat(10 * 1024);
    let huge_result = "y".repeat(10 * 1024);
    let input = tool_input(Some(&huge_args), Some(&huge_result));

    // Act
    let attributes = tool_span_attributes(&input);

    // Assert：长度上限 + 截断标记（标记措辞由实现定，只认「truncated / 截断」语义关键字）。
    for key in [KEY_INPUT, KEY_OUTPUT] {
        let value = attr_str(&attributes, key)
            .unwrap_or_else(|| panic!("P0-1：超长预览仍必须写出 {key}（截断而非丢弃）：{attributes:?}"));
        assert!(
            value.len() <= PREVIEW_LIMIT_BYTES,
            "P0-1：{key} 截断后长度必须 ≤ {PREVIEW_LIMIT_BYTES} 字节，实际 {}",
            value.len()
        );
        let lowered = value.to_lowercase();
        assert!(
            lowered.contains("truncat") || value.contains("截断"),
            "P0-1：{key} 截断后必须带截断语义标记（truncated/截断），实际尾部={:?}",
            &value[value.len().saturating_sub(40)..]
        );
        assert!(
            !value.trim().is_empty(),
            "P0-1：{key} 截断不得变成空串"
        );
    }
}

// ── 契约 3：敏感串必须脱敏 ────────────────────────────────────────────────

#[test]
fn sensitive_previews_are_sanitized() {
    let _gate = capture_lock();
    ensure_capture_default_on();

    // Arrange：假 token（`sk-` 前缀）+ 家目录路径。二者都在 `ponysentry::sanitize` 的脱敏面内：
    //  - JSON 敏感键值 `"api_key": "..."` ⇒ 整个值被替换；
    //  - `Bearer <token>` ⇒ 被替换；
    //  - `/home/<user>` ⇒ `[USER_HOME]`。
    const FAKE_TOKEN: &str = "sk-live-TOPSECRET-9876543210";
    // 裸 token（不带 JSON 键、不带 Bearer）：依赖 sanitize 的**新增裸 token 规则**脱敏。
    const BARE_SK: &str = "sk-abcdef1234567890abcdef";
    const BARE_GHP: &str = "ghp_0123456789abcdefghij0123456789abcdef"; // ghp_ + 36 位
    const BARE_AKIA: &str = "AKIAABCDEFGHIJKLMNOP"; // AKIA + 16 位大写

    let args = format!(
        r#"{{"api_key":"{FAKE_TOKEN}","note":"Bearer {FAKE_TOKEN}","path":"/home/dm/pony-agent/secret.ts","raw":"{BARE_SK} {BARE_GHP} {BARE_AKIA}"}}"#
    );
    // result 只放裸 token：这样该属性里一旦出现 [REDACTED_SECRET]，只能来自裸 token 脱敏。
    let result = format!("{BARE_SK} {BARE_GHP} {BARE_AKIA}");

    // 前置自检：这些串确实会被 sanitize 改写（否则本用例无鉴别力）。
    assert!(
        !sanitize(&args).contains(FAKE_TOKEN),
        "前置条件：args 中的假 token 必须会被 ponysentry::sanitize 脱敏"
    );
    for bare in [BARE_SK, BARE_GHP, BARE_AKIA] {
        assert!(
            !sanitize(&args).contains(bare),
            "前置条件：args 中的裸 token `{bare}` 必须会被 sanitize 脱敏（新增裸 token 规则）"
        );
        assert!(
            !sanitize(&result).contains(bare),
            "前置条件：result 中的裸 token `{bare}` 必须会被 sanitize 脱敏（新增裸 token 规则）"
        );
    }

    let input = tool_input(Some(&args), Some(&result));

    // Act
    let attributes = tool_span_attributes(&input);

    // Assert 1：所有敏感原串都不得出现（硬要求）。
    for key in [KEY_INPUT, KEY_OUTPUT] {
        let value = attr_str(&attributes, key)
            .unwrap_or_else(|| panic!("P0-1：{key} 必须写出（脱敏后）：{attributes:?}"));
        for raw in [FAKE_TOKEN, BARE_SK, BARE_GHP, BARE_AKIA] {
            assert!(
                !value.contains(raw),
                "P0-1：{key} 不得出现未脱敏敏感串（原串 `{raw}`），实际={value}"
            );
        }
        assert!(
            !value.contains("/home/dm"),
            "P0-1：{key} 不得出现未脱敏的家目录路径（原串 `/home/dm`），实际={value}"
        );
    }

    // Assert 2：脱敏标记必须出现（等价标记由实现定，此处按仓内既有标记断言）。
    // INPUT 侧含包装形态 + 裸 token；OUTPUT 侧**只含裸 token**，故其标记只能来自裸 token 脱敏。
    for key in [KEY_INPUT, KEY_OUTPUT] {
        let value = attr_str(&attributes, key)
            .unwrap_or_else(|| panic!("P0-1：{key} 必须写出（脱敏后）：{attributes:?}"));
        assert!(
            value.contains("[REDACTED_SECRET]"),
            "P0-1：{key} 必须出现脱敏标记 `[REDACTED_SECRET]`（裸 token 也要脱敏），实际={value}"
        );
    }
}

// ── 契约 4：None ⇒ 不写键 ─────────────────────────────────────────────────

#[test]
fn absent_previews_omit_keys_entirely() {
    let _gate = capture_lock();
    ensure_capture_default_on();

    // Arrange
    let input = tool_input(None, None);

    // Act
    let attributes = tool_span_attributes(&input);

    // Assert：键必须缺失（禁止空串占位）。
    assert!(
        attributes.iter().all(|(name, _)| name != KEY_INPUT),
        "P0-1：args 预览为 None 时不得写出 {KEY_INPUT}（含空串占位）；实际={attributes:?}"
    );
    assert!(
        attributes.iter().all(|(name, _)| name != KEY_OUTPUT),
        "P0-1：result 预览为 None 时不得写出 {KEY_OUTPUT}（含空串占位）；实际={attributes:?}"
    );
}

// ── 契约 5：开关关闭 ⇒ 两键都不出现 ───────────────────────────────────────

#[test]
fn capture_switch_disables_payload_keys() {
    let _gate = capture_lock();
    // Arrange：显式关闭 capture。
    let _env = EnvVarGuard::set(CAPTURE_ENV, "0");
    let input = tool_input(Some(r#"{"command":"echo hi"}"#), Some(r#"{"ok":true}"#));

    // Act
    let attributes = tool_span_attributes(&input);

    // Assert
    assert!(
        attributes.iter().all(|(name, _)| name != KEY_INPUT),
        "P0-1：{CAPTURE_ENV}=0 时不得写出 {KEY_INPUT}；实际={attributes:?}"
    );
    assert!(
        attributes.iter().all(|(name, _)| name != KEY_OUTPUT),
        "P0-1：{CAPTURE_ENV}=0 时不得写出 {KEY_OUTPUT}；实际={attributes:?}"
    );
    // 既有属性不受开关影响（关 payload ≠ 关 span）。
    assert!(
        attributes.iter().any(|(name, _)| name == "gen_ai.tool.name"),
        "P0-1：关闭 payload capture 不得影响既有工具属性；实际={attributes:?}"
    );
}
