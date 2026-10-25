pub mod client;
pub mod config;
pub mod models;
pub mod sanitizer;

pub use client::PonySentryClient;
pub use config::PonySentryConfig;
pub use models::{Breadcrumb, Exception, Frame, IngestPayload};
pub use sanitizer::{sanitize, sanitize_json};

use std::collections::{HashMap, HashSet};
use std::panic::PanicHookInfo;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock, RwLock};

static GLOBAL_CLIENT: OnceLock<RwLock<PonySentryClient>> = OnceLock::new();

fn get_global() -> &'static RwLock<PonySentryClient> {
    GLOBAL_CLIENT.get_or_init(|| {
        let cfg = PonySentryConfig::from_env();
        RwLock::new(PonySentryClient::new(cfg))
    })
}

pub fn init(config: PonySentryConfig) {
    let client = PonySentryClient::new(config);
    if let Some(lock) = GLOBAL_CLIENT.get() {
        if let Ok(mut write_guard) = lock.write() {
            *write_guard = client;
            return;
        }
    }
    let _ = GLOBAL_CLIENT.set(RwLock::new(client));
}

pub fn capture_error(error_type: &str, message: &str, extra: Option<serde_json::Value>) {
    if let Ok(client) = get_global().read() {
        client.capture_error(error_type, message, extra);
    }
}

pub fn capture_payload(payload: IngestPayload) {
    if let Ok(client) = get_global().read() {
        client.capture_payload(payload);
    }
}

pub fn capture_panic(info: &PanicHookInfo) {
    if let Ok(client) = get_global().read() {
        client.capture_panic(info);
    }
}

pub fn add_breadcrumb(
    category: &str,
    message: &str,
    data: Option<HashMap<String, String>>,
) {
    if let Ok(client) = get_global().read() {
        client.add_breadcrumb(category, message, data);
    }
}

// ------------------------------------------------ turn 失败上报（Wave-5 P0，CLI/TUI 专用）

/// 进程级开关：turn 失败是否上报 PonySentry（**缺省关闭**）。
///
/// 为什么默认关闭：桌面端已有前端 `TurnExecutionFailed` 上报链路，core 再报会**重复**；
/// 只有 CLI / TUI 两种「无前端」交付形态在启动引导里显式打开（`enable_turn_failure_capture(true)`）。
static TURN_FAILURE_CAPTURE: AtomicBool = AtomicBool::new(false);

/// 已上报过的 turn_id 集合（同一 turn 只报一次：防 fallback 重试 / 多出口重复上报）。
///
/// 有界：超过 [`REPORTED_TURN_CAPACITY`] 时整表清空（进程生命周期内最多泄漏这么多短字符串）。
static REPORTED_FAILURE_TURNS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

const REPORTED_TURN_CAPACITY: usize = 1024;

/// 打开/关闭「turn 失败上报」（仅 CLI/TUI 引导调用；桌面端保持关闭）。
pub fn enable_turn_failure_capture(enabled: bool) {
    TURN_FAILURE_CAPTURE.store(enabled, Ordering::SeqCst);
}

/// 当前是否开启 turn 失败上报。
pub fn is_turn_failure_capture_enabled() -> bool {
    TURN_FAILURE_CAPTURE.load(Ordering::SeqCst)
}

/// 上报一次 turn 失败（旁路：任何异常都不影响轮次返回）。
///
/// 调用方：`turn_flow::emit_stream_failed`（failed 终态唯一装配点）。
/// 语义：
/// - 开关关闭 ⇒ 立即返回（桌面端零开销、零重复上报）；
/// - **按 turn_id 去重**：同一 turn 只上报一次（fallback 重试/多出口不会多报）；
/// - payload 经既有 `capture_error` 链路：`tags.project=pony-agent`、`event_id`、
///   导出态 `trace_id`（无活跃导出 span 时为 `None`）、worker 侧补充栈/culprit；
/// - 失败吞掉：内部只走有界队列，绝不 panic、绝不冒泡。
pub fn capture_turn_failure(turn_id: &str, error: &str, model: Option<&str>) {
    if !is_turn_failure_capture_enabled() {
        return;
    }
    let turn_id = turn_id.trim();
    if turn_id.is_empty() {
        return;
    }
    if !claim_turn_failure_report(turn_id) {
        return; // 该 turn 已报过
    }
    // message 里带上 model 便于聚合检索；turn_id/model 同时进 extra 供详情页展示。
    let message = match model.map(str::trim).filter(|value| !value.is_empty()) {
        Some(model) => format!("[model={model}] {error}"),
        None => error.to_string(),
    };
    let extra = serde_json::json!({
        "turn_id": turn_id,
        "model": model,
        "source": "core_turn_failure",
    });
    capture_error("TurnExecutionFailed", &message, Some(extra));
}

/// 抢占「该 turn 的上报权」：返回 `true` 表示本次由我上报。
fn claim_turn_failure_report(turn_id: &str) -> bool {
    let mut guard = REPORTED_FAILURE_TURNS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.len() >= REPORTED_TURN_CAPACITY {
        guard.clear();
    }
    guard.insert(turn_id.to_string())
}

/// 排空上报队列（有界，最多 1500ms）：供 CLI/TUI 退出前调用，避免「报错即退出」丢 issue。
pub fn flush() {
    if let Ok(client) = get_global().read() {
        client.flush();
    }
}

pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        capture_panic(info);
        prev(info);
    }));
}

/// 检查 PonySentry 全局客户端是否处于启用状态
pub fn is_enabled() -> bool {
    if let Ok(client) = get_global().read() {
        client.config.enabled
    } else {
        false
    }
}


#[cfg(test)]
mod turn_failure_capture_tests {
    use super::*;

    /// 开关默认关闭（桌面端不重复上报）；可显式开关。
    #[test]
    fn turn_failure_capture_switch_defaults_off_and_roundtrips() {
        enable_turn_failure_capture(false);
        assert!(
            !is_turn_failure_capture_enabled(),
            "缺省必须关闭（桌面端由前端上报，避免重复 issue）"
        );
        enable_turn_failure_capture(true);
        assert!(is_turn_failure_capture_enabled());
        enable_turn_failure_capture(false);
        assert!(!is_turn_failure_capture_enabled());
    }

    /// 按 turn 去重：同一 turn 只抢占一次上报权（fallback 重试/多出口不会多报）。
    #[test]
    fn turn_failure_report_is_deduped_per_turn() {
        let turn = format!("turn-dedupe-{}", uuid::Uuid::new_v4());
        assert!(claim_turn_failure_report(&turn), "首次必须允许上报");
        assert!(!claim_turn_failure_report(&turn), "同一 turn 第二次必须被去重");
        // 不同 turn 互不影响。
        let other = format!("turn-dedupe-{}", uuid::Uuid::new_v4());
        assert!(claim_turn_failure_report(&other));
    }

    /// 空 turn_id 不占用上报权（畸形输入不污染去重表）。
    #[test]
    fn blank_turn_id_is_ignored_by_capture() {
        enable_turn_failure_capture(true);
        // 无 token 的默认全局客户端也是 fail-closed，这里只验证不 panic、不占表。
        capture_turn_failure("   ", "boom", Some("m"));
        capture_turn_failure("", "boom", None);
        enable_turn_failure_capture(false);
    }
}
