//! wave-18 L2-AT 对抗测试：PonySentry 凭证门禁
//!
//! 契约基线：`.dev-team/contract-matrix-wave18.md`（C1/C2/F1 + §7 迁移清单）
//!
//! ① 三处泄漏点源码静态零裸 64-hex 字面量（含前端 F1 第三处泄漏点）。
//! ② 畸形 / 边界 env 输入不得“复活”凭证，也不得 panic。
//! ③ 固定线程数 + `Barrier` 并发读写 `PONYSENTRY_*`，`from_env()` 不得 panic，
//!    且竞态结束后必须回到 fail-closed 默认态（无 sleep 概率等待，无调度运气依赖）。
//!
//! 红相基线：`from_env()` 在 `PONYSENTRY_CLIENT_TOKEN` 未设/空白时回落硬编码 64-hex 凭证。

use pony_agent_core::agent::ponysentry::PonySentryConfig;
use std::sync::{Arc, Barrier, Mutex, MutexGuard};

/// 同二进制内所有触碰 env 的用例串行化；并发用例内部自带 Barrier，两者叠加保证确定性。
static ENV_LOCK: Mutex<()> = Mutex::new(());

const MANAGED_ENV_KEYS: [&str; 4] = [
    "PONYSENTRY_CLIENT_TOKEN",
    "PONYSENTRY_ENABLED",
    "PONYSENTRY_INGEST_URL",
    "APP_ENV",
];

/// 对抗注入用的**测试虚拟凭证**（非真实凭证，长度刻意短于 64-hex 窗口）。
const INJECTED_TOKEN: &str = "test-token-churn";

const CHURN_THREADS: usize = 8;
const CHURN_ROUNDS: usize = 64;

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

/// 返回 `line` 中长度为 **恰好 64** 的裸 hex 连续段数量（长度窗口严格 64）。
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

// ---------------------------------------------------------------------------
// ① 静态：三处泄漏点零裸 64-hex 字面量
// ---------------------------------------------------------------------------

#[test]
fn wave18_leak_sites_contain_no_bare_64_hex_literal() {
    // Arrange：三处泄漏点中的两处源码站点（第三处为被迁移的测试断言副本，见
    // `ponysentry_telemetry_test.rs::test_config_contract`，由该文件自身断言覆盖）。
    let sites = [
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/agent/ponysentry/config.rs"
        ),
        // F1：前端第三处泄漏点（src/lib/telemetry.ts）
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/lib/telemetry.ts"),
    ];

    for site in sites {
        // Act
        let source = std::fs::read_to_string(site).unwrap_or_else(|err| {
            panic!("泄漏点源码必须可读（{site}）：{err}");
        });
        let offenders: Vec<usize> = source
            .lines()
            .enumerate()
            .filter(|(_, line)| bare_64_hex_runs(line) > 0)
            .map(|(index, _)| index + 1)
            .collect();

        // Assert
        assert!(
            offenders.is_empty(),
            "wave-18 凭证清理未闭环：{site} 仍含裸 64-hex 字面量，命中行号 {offenders:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// ② 畸形 / 边界 env 输入
// ---------------------------------------------------------------------------

#[test]
fn malformed_env_values_never_resurrect_credential() {
    let _guard = lock_env();

    // 空白族输入必须等价于“未设”，不得触发硬编码回落
    let blank_cases: [&str; 4] = ["", "   ", "\t", "\n"];
    for raw in blank_cases {
        clear_managed_env();
        std::env::set_var("PONYSENTRY_CLIENT_TOKEN", raw);
        let config = PonySentryConfig::from_env();
        assert!(
            config.client_token.is_none(),
            "畸形输入 PONYSENTRY_CLIENT_TOKEN={raw:?} 属空白族，必须视为未设 ⇒ None，实测 len={:?}",
            config.client_token.as_ref().map(|token| token.len())
        );
    }

    // 超长畸形 token：必须原样取自 env（不得被任何回落覆盖），且不得 panic
    clear_managed_env();
    let oversized = "t".repeat(4096);
    std::env::set_var("PONYSENTRY_CLIENT_TOKEN", &oversized);
    let config = PonySentryConfig::from_env();
    assert_eq!(
        config.client_token.as_deref().map(|token| token.len()),
        Some(4096),
        "显式超长 token 必须原样透传（唯一合法来源是 env）"
    );

    clear_managed_env();
}

#[test]
fn malformed_enabled_values_never_disable_or_flip_behavior() {
    let _guard = lock_env();
    clear_managed_env();

    // wave-18c：PONYSENTRY_ENABLED 变量已废除 —— 任何畸形取值都不得改变行为，
    // enabled 恒 true；零 egress 由 client_token 缺失承担（此处断言 token 仍为 None）。
    for raw in ["false", "FALSE", "False", " false ", "\tfalse\t", "0", " 0 ", ""] {
        std::env::set_var("PONYSENTRY_ENABLED", raw);
        let config = PonySentryConfig::from_env();
        assert!(
            config.enabled,
            "PONYSENTRY_ENABLED={raw:?} 不得影响行为（变量已废除，enabled 恒 true）"
        );
        assert!(
            config.client_token.is_none(),
            "PONYSENTRY_ENABLED={raw:?} 不得复活任何凭证回落（零 egress 由 token 承担）"
        );
    }

    clear_managed_env();
}

// ---------------------------------------------------------------------------
// ③ 并发 env 竞态（固定线程数 + Barrier）
// ---------------------------------------------------------------------------

#[test]
fn concurrent_env_churn_never_panics_and_stays_fail_closed() {
    // 与同二进制其它 env 用例互斥，避免外部干扰导致假红/假绿
    let _guard = lock_env();
    clear_managed_env();

    // Arrange：固定同步点，所有线程在 Barrier 放行后同时开始 churn
    let barrier = Arc::new(Barrier::new(CHURN_THREADS));
    let mut handles = Vec::with_capacity(CHURN_THREADS);

    for thread_id in 0..CHURN_THREADS {
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            for round in 0..CHURN_ROUNDS {
                match (thread_id + round) % 4 {
                    0 => std::env::set_var("PONYSENTRY_CLIENT_TOKEN", INJECTED_TOKEN),
                    1 => std::env::remove_var("PONYSENTRY_CLIENT_TOKEN"),
                    2 => std::env::set_var("PONYSENTRY_ENABLED", "false"),
                    _ => std::env::remove_var("PONYSENTRY_ENABLED"),
                }

                // Act：并发态下读取配置
                let config = PonySentryConfig::from_env();

                // Assert：结构性不变量 —— 不得 panic、字段回落链不得被竞态破坏
                assert!(
                    !config.endpoint.trim().is_empty(),
                    "并发竞态下 endpoint 不得为空"
                );
                assert!(
                    !config.environment.trim().is_empty(),
                    "并发竞态下 environment 不得为空"
                );

                if let Some(token) = config.client_token.as_deref() {
                    // 只允许来自显式注入；任何其它取值都意味着硬编码回落复活。
                    // 断言消息只暴露长度，不回显凭证本体。
                    assert!(
                        token == INJECTED_TOKEN,
                        "并发竞态下 client_token 出现非注入来源取值（疑似硬编码回落），len={}",
                        token.len()
                    );
                }
            }
        }));
    }

    // Act：等待全部线程退出（任何线程 panic 都会在此处冒泡）
    for handle in handles {
        handle
            .join()
            .expect("并发 env 竞态用例不得 panic（from_env 必须线程安全地不 panic）");
    }

    // Assert：竞态结束后清空 env，必须回到 fail-closed 默认态
    clear_managed_env();
    let settled = PonySentryConfig::from_env();
    assert!(
        settled.client_token.is_none(),
        "竞态收尾后 client_token 必须为 None（fail-closed），实测 len={:?}",
        settled.client_token.as_ref().map(|token| token.len())
    );
    assert!(
        settled.enabled,
        "竞态收尾后 enabled 恒为 true（wave-18c：开关变量已废除，fail-closed 由 token 承担）"
    );
}
