//! wave-18 L2-T 契约测试：PonySentry 凭证门禁 fail-closed 默认态
//!
//! 契约基线：`.dev-team/contract-matrix-wave18.md`
//!   - §1 C1：`PonySentryConfig::default()` ⇒ `client_token == None`、`enabled == false`；
//!     `endpoint` / `environment` / `release` 取值与回落链不变。
//!   - §2 C2：`from_env()` 的 token **仅**来自 `PONYSENTRY_CLIENT_TOKEN`
//!     （未设 / 纯空白 ⇒ `None`，禁止任何硬编码回落）；
//!     `enabled` 未设 ⇒ `false`；显式 `"false"` / `"0"`（含大小写与首尾空白）⇒ `false`，
//!     `"true"` / `"1"` ⇒ `true`。
//!   - §4 C1 冻结断言 4：`config.rs` 内不得残留任何裸 64-hex 字面量（静态负向断言）。
//!   - F4：`default()`（`enabled == false`）下 `capture_*` 必须零网络出口且不阻塞调用方，
//!     不得退化为“只断言不 panic”的恒真空断言。
//!
//! 红相基线（迁移前）：`default().client_token == Some(<硬编码 64-hex>)`、`default().enabled == true`；
//! `from_env()` 在 `PONYSENTRY_CLIENT_TOKEN` 未设/空白时回落同一硬编码 token；
//! `PONYSENTRY_ENABLED` 未设时 `.unwrap_or(true)`。
//!
//! 确定性：`std::env` 是进程级全局状态，本文件所有触碰 env 的用例统一经 `ENV_LOCK`
//! 串行化，不依赖测试线程调度运气。

use pony_agent_core::agent::ponysentry::{IngestPayload, PonySentryClient, PonySentryConfig};
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// 同二进制内 env 用例的串行锁（跨二进制由 cargo 顺序执行测试目标保证）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 本契约涉及的 env 键全集；每个用例前后都必须清空，避免宿主环境泄漏进断言。
const MANAGED_ENV_KEYS: [&str; 5] = [
    "PONYSENTRY_CLIENT_TOKEN",
    "PONYSENTRY_ENABLED",
    "PONYSENTRY_INGEST_URL",
    "APP_ENV",
    "APP_RELEASE",
];

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn clear_managed_env() {
    for key in MANAGED_ENV_KEYS {
        std::env::remove_var(key);
    }
}

/// 返回 `line` 中长度为 **恰好 64** 的裸 hex 连续段数量。
///
/// 与门禁规则 `(?<![0-9a-fA-F])[0-9a-fA-F]{64}(?![0-9a-fA-F])` 的行级语义等价：
/// 63 / 65 / 128 位连续 hex 段均不计入（长度窗口严格 64）。
fn bare_64_hex_runs(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut hits = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_hexdigit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                i += 1;
            }
            if i - start == 64 {
                hits += 1;
            }
        } else {
            i += 1;
        }
    }
    hits
}

/// 显式条件轮询（带截止时间）：用于判定本地回环探针是否收到过 TCP 连接。
/// 采用固定轮询间隔 + 硬截止时间，禁止无界等待，也不依赖单次 sleep 的“概率命中”。
fn wait_for_egress(listener: &TcpListener, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        match listener.accept() {
            Ok(_stream) => return true,
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return false,
        }
    }
    false
}

/// 最小合法 IngestPayload（仅用于触发上报路径，不含任何敏感字段）。
fn minimal_ingest_payload() -> IngestPayload {
    IngestPayload {
        platform: "rust".to_string(),
        release: "wave18-egress-probe".to_string(),
        environment: "test".to_string(),
        message: Some("wave18 f4 egress probe".to_string()),
        exception: None,
        tags: None,
        extra: None,
        breadcrumbs: None,
    }
}

// ---------------------------------------------------------------------------
// C1：default()
// ---------------------------------------------------------------------------

#[test]
fn default_config_is_fail_closed() {
    // Arrange / Act
    let config = PonySentryConfig::default();

    // Assert
    assert!(
        config.client_token.is_none(),
        "C1：default().client_token 必须为 None（禁止任何硬编码凭证回落），实测 len={:?}",
        config.client_token.as_ref().map(|token| token.len())
    );
    assert!(
        !config.enabled,
        "C1：default().enabled 必须为 false（默认零 egress / fail-closed）"
    );
    assert_eq!(config.endpoint, "https://sentry.ponyjob.top");
    assert_eq!(config.environment, "dev");
    assert_eq!(config.release, env!("CARGO_PKG_VERSION"));
}

// ---------------------------------------------------------------------------
// C2：from_env()
// ---------------------------------------------------------------------------

#[test]
fn from_env_without_credentials_is_fail_closed() {
    let _guard = lock_env();
    clear_managed_env();

    // Act
    let config = PonySentryConfig::from_env();

    // Assert：未设 env ⇒ 既无 token 也不启用，绝不回落到硬编码凭证
    assert!(
        config.client_token.is_none(),
        "C2-1：PONYSENTRY_CLIENT_TOKEN 未设时 client_token 必须为 None（禁止回落），实测 len={:?}",
        config.client_token.as_ref().map(|token| token.len())
    );
    assert!(
        !config.enabled,
        "C2-3：PONYSENTRY_ENABLED 未设时 enabled 必须为 false"
    );
    assert_eq!(config.endpoint, "https://sentry.ponyjob.top");
    assert_eq!(config.environment, "dev");
}

#[test]
fn from_env_blank_token_is_treated_as_unset() {
    let _guard = lock_env();
    clear_managed_env();

    // Arrange / Act
    std::env::set_var("PONYSENTRY_CLIENT_TOKEN", "   ");
    let config = PonySentryConfig::from_env();

    // Assert
    assert!(
        config.client_token.is_none(),
        "C2-2：纯空白 PONYSENTRY_CLIENT_TOKEN 必须等价于未设 ⇒ None，实测 len={:?}",
        config.client_token.as_ref().map(|token| token.len())
    );

    clear_managed_env();
}

#[test]
fn from_env_explicit_token_is_honored_verbatim() {
    let _guard = lock_env();
    clear_managed_env();

    // Arrange
    std::env::set_var("PONYSENTRY_CLIENT_TOKEN", "test-token-123");

    // Act
    let config = PonySentryConfig::from_env();

    // Assert：显式注入是 token 的唯一合法来源
    assert_eq!(config.client_token.as_deref(), Some("test-token-123"));

    clear_managed_env();
}

#[test]
fn from_env_enabled_resolution_matrix() {
    let _guard = lock_env();
    clear_managed_env();

    // 未设 ⇒ false（fail-closed）
    let unset = PonySentryConfig::from_env();
    assert!(
        !unset.enabled,
        "C2-3：PONYSENTRY_ENABLED 未设时必须解析为 false"
    );

    // 显式真值表（保留 config.rs 既有解析表达式语义）
    let cases: [(&str, bool); 8] = [
        ("false", false),
        ("FALSE", false),
        (" false ", false),
        ("0", false),
        ("  0  ", false),
        ("true", true),
        ("TRUE", true),
        (" true ", true),
    ];
    for (raw, expected) in cases {
        std::env::set_var("PONYSENTRY_ENABLED", raw);
        let config = PonySentryConfig::from_env();
        assert_eq!(
            config.enabled, expected,
            "C2-4/5：PONYSENTRY_ENABLED={raw:?} 的真值解析不符合冻结契约"
        );
    }

    clear_managed_env();
}

/// wave18-r3 §2 冻结断言 7/8/9（C2 fail-closed 边界，`review-l3-contract` finding#4）。
///
/// `PONYSENTRY_ENABLED` **set-but-empty / 纯空白**必须与 unset 同义 ⇒ `false`。
/// 红相基线：`config.rs` 的
/// `.map(|s| s.trim().to_lowercase() != "false" && s.trim() != "0").unwrap_or(false)`
/// 使 `Some("")` / `Some("   ")` / `Some("\t")` 落进 `map` 闭包 ⇒ **true**（fail-open 缺口）。
#[test]
fn from_env_enabled_set_but_empty_is_fail_closed() {
    let _guard = lock_env();
    clear_managed_env();

    // §2 断言 7：set-but-empty / 纯空白 ⇒ false（与 unset 同义）
    let blank_cases: [(&str, &str); 4] = [
        ("空串", ""),
        ("空格串", "   "),
        ("制表符", "\t"),
        ("换行符", "\n"),
    ];
    for (label, raw) in blank_cases {
        std::env::set_var("PONYSENTRY_ENABLED", raw);
        let config = PonySentryConfig::from_env();
        assert!(
            !config.enabled,
            "C2-r3 断言 7（fail-closed）：PONYSENTRY_ENABLED 为{label}（set-but-empty）时必须为 false，\
             不得因落进 map 闭包而 fail-open 为 true"
        );
    }

    // §2 断言 8：trim 先行 —— 空白包裹不得改变显式值语义
    let padded_cases: [(&str, &str, bool); 4] = [
        ("空白包裹 0", "  0  ", false),
        ("空白包裹 false", " false ", false),
        ("空白包裹 true", " true ", true),
        ("制表符包裹 1", "\t1\t", true),
    ];
    for (label, raw, expected) in padded_cases {
        std::env::set_var("PONYSENTRY_ENABLED", raw);
        let config = PonySentryConfig::from_env();
        assert_eq!(
            config.enabled, expected,
            "C2-r3 断言 8：PONYSENTRY_ENABLED 为{label}（{raw:?}）时必须解析为 {expected}（先 trim 再判）"
        );
    }

    clear_managed_env();
}

// ---------------------------------------------------------------------------
// §4 C1 冻结断言 4：静态负向（源码零裸 64-hex 字面量）
// ---------------------------------------------------------------------------

#[test]
fn config_source_contains_no_bare_64_hex_literal() {
    // Arrange
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/agent/ponysentry/config.rs"
    );
    let source = std::fs::read_to_string(path).expect("config.rs 必须存在");

    // Act
    let offenders: Vec<usize> = source
        .lines()
        .enumerate()
        .filter(|(_, line)| bare_64_hex_runs(line) > 0)
        .map(|(index, _)| index + 1)
        .collect();

    // Assert
    assert!(
        offenders.is_empty(),
        "C1-4：config.rs 不得残留任何裸 64-hex 凭证字面量，命中行号 {offenders:?}"
    );
}

// ---------------------------------------------------------------------------
// F4：默认禁用态必须零网络出口且不阻塞
// ---------------------------------------------------------------------------

#[test]
fn default_config_yields_zero_egress_and_never_blocks() {
    // Arrange：本地回环监听器作为“网络出口探针”。
    // 仅把 endpoint 指向探针，其余字段全部取自 default()，因此 enabled / token
    // 完全由 default() 决定——这正是 F4 的被测点。
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback egress probe");
    listener
        .set_nonblocking(true)
        .expect("egress probe must be non-blocking");
    let port = listener.local_addr().expect("probe local_addr").port();

    let mut config = PonySentryConfig::default();
    config.endpoint = format!("http://127.0.0.1:{port}");
    let client = PonySentryClient::new(config);

    // Act：fire-and-forget 语义下调用方不得被阻塞
    let started = Instant::now();
    client.capture_error("F4EgressProbe", "default config egress probe", None);
    client.capture_payload(minimal_ingest_payload());
    let elapsed = started.elapsed();

    // Assert 1：不阻塞
    assert!(
        elapsed < Duration::from_millis(100),
        "F4：default() 下 capture_* 阻塞了 {elapsed:?}（fail-closed 必须零成本早退）"
    );

    // Assert 2：零网络出口（3s 探针窗口内不得出现任何 TCP 连接）
    assert!(
        !wait_for_egress(&listener, Duration::from_secs(3)),
        "F4：default() 必须 enabled=false ⇒ 零 egress，但 capture_* 仍向 127.0.0.1:{port} 发起了连接"
    );
}
