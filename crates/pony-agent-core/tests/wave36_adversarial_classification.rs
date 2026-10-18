//! Wave-36 / L2-AT 对抗破坏测试：上游错误分类的确定性注入面（PA-143 / PA-144 / PA-145）。
//!
//! 关注点不是「正常文本能被识别」，而是「与原因无关的字节（长数字串片段、端口、多字节边界、
//! 转义引号、超长 payload）不得改变分类结论」，且真实原因码不得被过度收紧。
//!
//! 只消费公开 API：
//! - `pony_agent_core::agent::retry::{ProviderRetryPolicy, is_rate_limit_error}`（PA-143/144 单一出口）
//! - `pony_agent_core::agent::recovery::{ExecutionRecoveryPolicy, RecoveryAction}`（PA-145 平行判据）
//!
//! 确定性约定：全部为固定字面量输入，无 `sleep`、无随机数、无非确定性并发、无网络；
//! 唯一的计时断言（1MB payload < 5s）给出 3 个数量级余量，不构成 flake 面。
//!
//! 契约锚点：`.dev-team/contracts/wave-36-pa138-145.md`
//! - §6 C-143-1/2/4/5：裸子串码表 ⇒ 更长数字串内片段不得伪造命中，真实码不得丢失；
//! - §7 C-144-1：scheme-less `host:port` 端口（含端口恰为码值 `:429`/`:502`）不得驱动分类；
//! - §8 C-145-3：recovery L1 决策必须与 `retry::classify` 同源一致。

use std::time::{Duration, Instant};

use pony_agent_core::agent::recovery::{
    ExecutionRecoveryPolicy, RecoveryAction, RecoveryContext, RecoveryLayer, RecoveryPipelineConfig,
};
use pony_agent_core::agent::retry::{
    is_rate_limit_error, BackoffConfig, ProviderRetryPolicy, RetryBudget,
};

fn retry_policy() -> ProviderRetryPolicy {
    ProviderRetryPolicy::new(BackoffConfig::default())
}

/// retry 单一出口：文本是否被判为瞬态可重试（`FailureKind::TransientRetryable`）。
fn retry_transient(text: &str) -> bool {
    retry_policy().classify(text).is_retryable()
}

/// `(是否瞬态可重试, 是否触发限流长退避)` 二元签名；用于「注入不得改变分类」的等值断言，
/// 不比较 `details` 字段（其内容刻意保留原始 message，必然随文本不同）。
fn signature(text: &str) -> (bool, bool) {
    (retry_transient(text), is_rate_limit_error(text))
}

/// 真实原因码：必须仍被判为瞬态（防「过度收紧」）。
fn assert_transient(text: &str, why: &str) {
    let kind = retry_policy().classify(text);
    assert!(
        kind.is_retryable(),
        "判据被过度收紧：真实原因码必须仍命中; why={why}; text={text:?}; kind={kind:?}"
    );
}

/// 污染文本：不得判为瞬态，且不得触发限流长退避。
fn assert_not_transient(text: &str, why: &str) {
    let kind = retry_policy().classify(text);
    assert!(
        !kind.is_retryable(),
        "无关数字/片段伪造了可重试分类; why={why}; text={text:?}; kind={kind:?}"
    );
    assert!(
        !is_rate_limit_error(text),
        "无关数字/片段伪造了限流长退避判定; why={why}; text={text:?}"
    );
}

/// 构造**只受 L1 瞬态判据影响**的恢复上下文：无 stream_state（跳过流降级分支）、
/// 预算未耗尽、无 fallback 模型（跳过 529 倒换分支）——与冻结测试
/// `tests/wave36_recovery_retry_consistency.rs` 使用同一公开入口与同一构造口径。
fn recovery_action(raw_error: &str) -> RecoveryAction {
    let policy = ExecutionRecoveryPolicy::new(RecoveryPipelineConfig::default());
    let budget = RetryBudget::new(3, 10_000);
    let ctx = RecoveryContext {
        layer: RecoveryLayer::L1TransportUpstream,
        raw_error: raw_error.to_string(),
        error_code: None,
        stream_state: None,
        current_attempt: 0,
        consecutive_failures: 1,
        primary_model: Some("claude-3-5-sonnet".to_string()),
        fallback_model: None,
        tool_name: None,
        tool_is_idempotent: true,
    };
    policy.decide(&ctx, &budget)
}

fn recovery_transient(raw_error: &str) -> bool {
    matches!(
        recovery_action(raw_error),
        RecoveryAction::RetryWithDelay { .. }
    )
}

// ---------------------------------------------------------------------------
// 1. 畸形 / 超长 payload：1MB 纯数字串中嵌入码值片段
// ---------------------------------------------------------------------------

/// PA-143 对抗面：码表若是裸 `contains`，1MB 数字串里的 `429`/`408`/`502` 三个片段任一
/// 都会把「与原因无关的超长数字」判成瞬态，且真实生产可达（错误正文直接拼接上游文本）。
/// 同时锁性能：分类必须是单次线性扫描量级，1MB 输入 < 5s（真断言，非注释）。
#[test]
fn t01_megabyte_digit_payload_must_not_forge_transient_classification() {
    // Arrange：1MB 数字串 + 嵌入片段 + 100 位后缀（两侧都是数字 ⇒ 片段不是独立 token）。
    let payloads: Vec<(&str, String)> = ["429", "408", "502"]
        .iter()
        .map(|code| {
            (
                *code,
                format!("{}{}{}", "9".repeat(1_000_000), code, "9".repeat(100)),
            )
        })
        .collect();

    // Act + Assert：逐个分类并计时。
    let started = Instant::now();
    for (code, payload) in &payloads {
        assert_eq!(
            payload.len(),
            1_000_000 + 3 + 100,
            "payload 构造必须是确定的 1MB 级输入"
        );
        let kind = retry_policy().classify(payload);
        assert!(
            !kind.is_retryable(),
            "1MB 数字串内嵌 {code} 不得伪造可重试分类; kind={kind:?}"
        );
        assert!(
            !is_rate_limit_error(payload),
            "1MB 数字串内嵌 {code} 不得伪造限流长退避判定"
        );
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "1MB payload 的分类耗时必须 < 5s（真断言）; actual={elapsed:?}"
    );

    // 对照：同样是 1MB 级输入，但码值是独立 token ⇒ 仍必须命中（防一刀切裁剪）。
    let with_token = format!("{} HTTP 429", "9".repeat(1_000_000));
    assert_transient(&with_token, "1MB 数字串之后的独立 `HTTP 429` token");
}

// ---------------------------------------------------------------------------
// 2. 多字节边界 + 端口不变性
// ---------------------------------------------------------------------------

/// PA-144 + 波次 G F1 对抗面：`str::find` 返回的字节索引不得做 `+1/-1` 偏移（会把边界切进
/// CJK/emoji/全角冒号内部 ⇒ `byte index N is not a char boundary` panic）；且端口数字不得
/// 改变分类（对比同 host、端口数字不同的两段文本）。
#[test]
fn t02_multibyte_boundaries_are_safe_and_port_digits_do_not_drive_classification() {
    // Arrange：URL 紧贴 CJK / emoji / 全角冒号，以及全角冒号后的 scheme-less host:port。
    let probes = [
        "访问https://example.com:42953/v1",
        "访问https://example.com:50208/v1/chat/completions",
        "错误：127.0.0.1:42953/v1", // U+FF1A 全角冒号紧贴 host:port
        "🔒localhost:50208",
        "🔒127.0.0.1:40891/v1",
        "请求失败(网络层)：https://gw.internal:50417/v1",
        "访问https://example.com/v1", // 无端口，作为「无污染」对照
    ];

    // Act + Assert：不 panic（panic 即失败）+ 不得因端口数字被分类为瞬态。
    for probe in probes {
        assert_not_transient(probe, "多字节紧贴 URL / scheme-less host:port 的端口数字");
    }

    // 端口不变性：同一 host 只改端口数字 ⇒ 分类签名必须完全相同。
    let pairs = [
        (
            "访问https://example.com:42953/v1",
            "访问https://example.com:12345/v1",
        ),
        ("错误：127.0.0.1:42953/v1", "错误：127.0.0.1:12345/v1"),
        ("🔒localhost:50208", "🔒localhost:12345"),
        ("🔒127.0.0.1:40891/v1", "🔒127.0.0.1:12345/v1"),
    ];
    for (polluted, benign) in pairs {
        assert_eq!(
            signature(polluted),
            signature(benign),
            "仅端口数字不同就不得改变分类; polluted={polluted:?}; benign={benign:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. 结构化数字串：纯错误码不是判据
// ---------------------------------------------------------------------------

/// PA-143 C-143-1/2：`elapsed=40832ms`、`"code":"429001"`、`retry_after=1429ms` 这类
/// 「更长数字串内的码值片段」不得改变分类。
#[test]
fn t03_structured_numeric_fields_are_not_code_evidence() {
    let probes = [
        ("provider stream aborted; elapsed=40832ms", "耗时数字 40832"),
        ("{\"code\":\"429001\"}", "JSON 错误码 429001"),
        ("{\"code\":\"1502\"}", "JSON 错误码 1502"),
        ("{\"error\":{\"code\":1503}}", "JSON 错误码 1503（数字形态）"),
        ("retry_after=1429ms", "重试提示数字 1429"),
        ("elapsed=340829ms", "耗时数字 340829"),
        ("{\"code\":\"429001\",\"message\":\"invalid request\"}", "错误码+文案混合"),
    ];
    for (probe, why) in probes {
        assert_not_transient(probe, why);
    }

    // 反向对照：同一 JSON 形态但 code 恰为独立码值 ⇒ 必须命中（结构化判定不得退化为「忽略所有 JSON」）。
    assert_transient(
        "{\"code\":\"429\",\"message\":\"rate limited\"}",
        "JSON 中 code 恰为 429 的独立 token",
    );
}

// ---------------------------------------------------------------------------
// 4. 边界值：独立 token 必须命中；更长数字串不得命中
// ---------------------------------------------------------------------------

/// PA-143 C-143-4 反向保护：词边界/结构化判定不得把真实码一起裁掉。
#[test]
fn t04_standalone_code_tokens_still_hit() {
    let hits = [
        ("HTTP 429", "空格分隔的 429"),
        ("status=429;", "`=` 与 `;` 分隔的 429"),
        ("code=429\n", "换行结尾的 429"),
        ("429 Too Many Requests", "429 位于行首"),
        ("HTTP 408", "408"),
        ("502 Bad Gateway", "502 位于行首"),
        ("HTTP 503", "503"),
        ("HTTP 504 Gateway Timeout", "504"),
        ("operation timed out", "timeout 措辞表"),
    ];
    for (probe, why) in hits {
        assert_transient(probe, why);
    }

    // 数字串边界：`4290` / `0429` 是更长数字（前导 0 也算更长）⇒ 不是独立 token。
    let misses = [
        ("4290", "尾随数字"),
        ("0429", "前导 0"),
        ("{\"code\":\"4290\"}", "JSON 尾随数字"),
        ("{\"code\":\"0429\"}", "JSON 前导 0"),
        ("HTTP 4290", "空格分隔但数字更长"),
        ("id=0429;", "前缀 0 且带分隔符"),
    ];
    for (probe, why) in misses {
        assert_not_transient(probe, why);
    }
}

// ---------------------------------------------------------------------------
// 5. 数字紧邻 ASCII 字母：词边界语义
// ---------------------------------------------------------------------------

/// PA-143 C-143-5「词边界判定」取向：正则 `\b429\b` 语义下，`429abc` / `abc429` 都是
/// **一个完整的词 token**（`\w` 含字母与数字），`429` 不落在词边界上，因此不构成限流/瞬态证据。
///
/// 语义歧义登记（本用例是唯一受取向影响的断言）：§6 C-143-5 同时允许「结构化/按错误码字段
/// 分类」，若实现改为「提取最长数字串并等值比较」取向，`429abc` 仍会命中、`abc429` 仍会命中 ⇒
/// 本用例将保持红。届时属**语义取向分歧**（须 Lead 裁决），不是分类缺陷；其余用例与
/// `4290`/`0429`/`40832`/`429001` 的判定在两种取向下结论相同。
#[test]
fn t05_digit_adjacent_to_ascii_letters_is_not_a_word_boundary_match() {
    let probes = [
        ("429abc", "数字后紧邻字母"),
        ("abc429", "字母后紧邻数字"),
        ("429abc; status", "紧邻字母后接分隔符"),
        ("error_429x", "下划线词内数字+字母"),
    ];
    for (probe, why) in probes {
        assert_not_transient(probe, why);
    }

    // 反向对照：独立 token 仍命中（避免用「禁止字母」一刀切掉合法命中）。
    assert_transient("HTTP 429abc-split 429", "独立 429 token 与紧邻字母 token 并存");
}

// ---------------------------------------------------------------------------
// 6. 转义注入
// ---------------------------------------------------------------------------

/// 转义/嵌套注入：`\"` 转义、裸 `}\n{` 拼接、2 万层引号嵌套都不得 panic，
/// 且不得把**更长数字串**（429001）变成命中；转义不得翻转分类结论。
#[test]
fn t06_escape_injection_is_stable_and_does_not_forge_hits() {
    // (a) 转义引号不得翻转结论：`"code":"429\"}"` 与未转义形态必须同签名。
    assert_eq!(
        signature(r#""code":"429\"}""#),
        signature(r#""code":"429""#),
        "转义引号不得翻转分类结论"
    );

    // (b) 更长数字串仍是更长数字串：转义不得让它命中。
    for probe in [
        r#""code":"429001\"}""#,
        r#""code":"1502\"}""#,
        "}\n{\"code\":429001",
        "}\\n{\"code\":429001",
        "{\"code\":429001}\n{\"code\":429001}",
    ] {
        assert_not_transient(probe, "转义/拼接后的更长数字串");
    }

    // (c) 超长引号嵌套：不得 panic（栈式扫描实现不得递归爆栈），且不得伪造命中。
    let nested = format!("{}\"code\":429001", "\"".repeat(20_000));
    assert_eq!(nested.len(), 20_000 + 13);
    assert_not_transient(&nested, "2 万层引号嵌套后的更长数字串");
    let nested_hit = format!("{}\"code\":\"429\"", "\"".repeat(20_000));
    assert_transient(&nested_hit, "2 万层引号嵌套后的独立 429 token");
}

// ---------------------------------------------------------------------------
// 7. scheme-less host:port 变体
// ---------------------------------------------------------------------------

/// PA-144 C-144-1：`strip_endpoint_fragments` 只剥 `://`，无 scheme 的 `host:port` 会把端口
/// 数字暴露给码表。端口**恰为码值**（`:429`/`:502`）时纯词边界仍会漏 ⇒ 必须按 host:port 结构剥离。
/// 覆盖 IPv4 / 域名 / localhost / bracketed IPv6。
#[test]
fn t07_schemeless_host_port_does_not_drive_classification() {
    let probes = [
        "endpoint=127.0.0.1:429/v1",
        "endpoint=127.0.0.1:42953/v1",
        "endpoint=localhost:502",
        "endpoint=localhost:50208",
        "endpoint=[::1]:42953/v1",
        "endpoint=[::1]:502",
        "endpoint=foo.internal:8080/path",
        "base_url=10.0.0.7:40891/v1/chat/completions",
    ];
    for probe in probes {
        assert_not_transient(probe, "scheme-less host:port 端口数字");
    }

    // 端口不变性（IPv4 / localhost / IPv6 三形态）。
    let pairs = [
        ("endpoint=127.0.0.1:429/v1", "endpoint=127.0.0.1:12345/v1"),
        ("endpoint=localhost:502", "endpoint=localhost:12345"),
        ("endpoint=[::1]:42953/v1", "endpoint=[::1]:12345/v1"),
        (
            "endpoint=foo.internal:8080/path",
            "endpoint=foo.internal:12345/path",
        ),
    ];
    for (polluted, benign) in pairs {
        assert_eq!(
            signature(polluted),
            signature(benign),
            "仅端口数字不同就不得改变分类（scheme-less host:port）; polluted={polluted:?}; benign={benign:?}"
        );
    }

    // 反向对照：不得把 host:port 之后的原因文本一起剥掉。
    assert_transient(
        "connection refused; endpoint=127.0.0.1:42953/v1",
        "host:port 之前的原因文本必须保留",
    );
}

// ---------------------------------------------------------------------------
// 8. recovery L1 与 retry 单一出口一致性
// ---------------------------------------------------------------------------

/// PA-145 C-145-3：`recovery.rs` 的第二套裸 `contains` 码表必须与 `retry::classify` 同源。
/// 该用例与冻结测试 `wave36_recovery_retry_consistency.rs` 使用同一公开入口，但把对抗面扩展
/// 到 PA-143/144 的注入语料 + 双向漂移探针。
#[test]
fn t08_recovery_l1_agrees_with_retry_single_exit_on_adversarial_corpus() {
    // A. 真实瞬态：两个出口都必须报瞬态（收敛时不得过度收紧）。
    for text in [
        "operation timed out",
        "HTTP 429 Too Many Requests",
        "HTTP 502 Bad Gateway",
        "connection reset by peer",
    ] {
        let action = recovery_action(text);
        assert!(
            matches!(action, RecoveryAction::RetryWithDelay { .. }),
            "recovery L1 必须报告瞬态: {text:?} -> {action:?}"
        );
        assert!(
            retry_transient(text),
            "retry 单一出口必须报告瞬态: {text:?}"
        );
    }

    // B. 污染文本（含 PA-143/144 注入面）：两个出口都必须 Abort / 非瞬态。
    for text in [
        "provider stream aborted; elapsed=40832ms",
        "{\"error\":{\"message\":\"invalid request\",\"code\":\"429001\"}}",
        "解析 provider SSE 流失败: 未找到 SSE data 事件; endpoint=127.0.0.1:42953/v1",
        "解析 provider SSE 流失败: 未找到 SSE data 事件; endpoint=localhost:50208",
        "解析 provider SSE 流失败: 未找到 SSE data 事件; endpoint=[::1]:42953/v1",
        "解析 provider SSE 流失败: 未找到 SSE data 事件; endpoint=127.0.0.1:429/v1",
        "访问https://example.com:42953/v1",
        "retry_after=1429ms",
    ] {
        let action = recovery_action(text);
        assert!(
            matches!(action, RecoveryAction::Abort { .. }),
            "recovery L1 对污染文本必须 Abort（不得因端口/长数字命中瞬态表）: {text:?} -> {action:?}"
        );
        assert!(
            !retry_transient(text),
            "retry 单一出口对污染文本必须非瞬态: {text:?}"
        );
    }

    // C. 双向漂移探针：只断言两出口**结论一致**，不预设收敛方向（避免替实现选边；
    //    529/500 的取向由 C-145-5 的报告义务约束，不在本对抗面定型）。
    for text in [
        "HTTP 408 Request Timeout",
        "HTTP 500 Internal Server Error",
        "HTTP 504 Gateway Timeout",
        "status:503",
        "operation timed out; endpoint=127.0.0.1:42953/v1",
        "{\"code\":\"4290\"}",
        "abc429",
    ] {
        assert_eq!(
            recovery_transient(text),
            retry_transient(text),
            "同一错误文本在 recovery L1 与 retry 单一出口结论必须一致（平行 contains 判据漂移）: {text:?} -> recovery={:?}",
            recovery_action(text)
        );
    }
}
