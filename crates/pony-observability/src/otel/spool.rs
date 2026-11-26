//! B202：OTLP 导出失败的**有界本地 spool** + 失败类别判别。
//!
//! 问题（`tui-runtime.log` 12:20–12:59Z 实测）：`otel.ponyjob.top` 长期 `TimedOut` 时，
//! 每批 span 在 2 次尝试后被直接丢弃，只剩一行「永久丢失」——**告警诊断链自身缺证据**
//! （`TraceSpanError` 告警无法回溯到具体 span）。
//!
//! 取舍：不做 OTLP 重放。`SpanData → protobuf ResourceSpans` 的 transform 是
//! `opentelemetry-otlp` 私有实现，绕过去等于自己维护一份协议编码。这里只把**失败批次的可诊断
//! 摘要**（时间 / 失败类别 / span 数 / 错误摘要）落本地有界文件，回答三个问题：
//! 丢了什么、为什么丢、是不是出口网络问题。
//!
//! 有界性：行数上限 + TTL 双闸（写前剪枝），避免黑洞端点下无界增长。

use std::io::Write as _;
use std::path::PathBuf;

/// spool 文件保留的最大记录行数（超出保留最新）。
///
/// 取值依据：一次导出失败通常对应一批 1–8 个 span，单行 ≤ ~400 字节；200 行 ≈ 80KB 量级，
/// 足以覆盖一整天的抖动期，又不会让 `PonyAgent/` 数据目录膨胀。
pub const SPOOL_MAX_LINES: usize = 200;

/// 单条记录的保留时长（24h）：跨天排查仍可读，过期即剪枝。
pub const SPOOL_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// 错误摘要的截断长度（避免把长 body 原样落盘）。
const ERROR_PREVIEW_CHARS: usize = 240;

/// 导出失败类别：**先分类再措辞**——出口网络问题与配置/鉴权问题对用户的下一步完全不同。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFailureKind {
    /// 出口网络不可达 / 超时 / DNS / 连接失败：本机──公网这一段的问题。
    Network,
    /// 鉴权失败（401/403/token）：凭据问题，需要重新配置。
    Auth,
    /// 配置 / 端点 / 编码问题（404、invalid、decode/parse）：采集端配置问题。
    Config,
    /// 其他（未识别）：保留原文摘要，不下结论。
    Other,
}

impl ExportFailureKind {
    /// 面向人读的一行类别名（写进日志与 spool，不承载 UI 样式）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Network => "出口网络不可达/超时",
            Self::Auth => "鉴权失败",
            Self::Config => "配置/端点不可用",
            Self::Other => "未识别失败",
        }
    }

    /// wire 名（写进 spool JSON，便于外部脚本按类别聚合）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Auth => "auth",
            Self::Config => "config",
            Self::Other => "other",
        }
    }
}

/// 由错误文本判定失败类别（纯函数，便于单测与复用）。
///
/// 判定顺序刻意「网络 → 鉴权 → 配置 → 其他」：`TimedOut` 常与 `connect` 同时出现，
/// 必须先认网络，否则会被误判成配置问题而把用户引向错误的排查方向。
pub fn classify_export_error(message: &str) -> ExportFailureKind {
    let lower = message.to_ascii_lowercase();
    let network = [
        "timedout",
        "timed out",
        "timeout",
        "connect",
        "dns",
        "network",
        "unreachable",
        "connection reset",
        "broken pipe",
    ];
    if network.iter().any(|hint| lower.contains(hint)) {
        return ExportFailureKind::Network;
    }
    let auth = ["401", "403", "unauthor", "forbidden", "invalid token", "api key"];
    if auth.iter().any(|hint| lower.contains(hint)) {
        return ExportFailureKind::Auth;
    }
    let config = [
        "404",
        "invalid",
        "decode",
        "parse",
        "config",
        "unsupported",
        "not found",
    ];
    if config.iter().any(|hint| lower.contains(hint)) {
        return ExportFailureKind::Config;
    }
    ExportFailureKind::Other
}

/// 当前毫秒时间戳（唯一时间源；导出线程与测试共用，避免各写一份 clock 读取）。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

/// 一条 spool 记录（单行 JSON；字段刻意扁平，便于 `jq` 直接消费）。
pub fn failure_record_line(
    kind: ExportFailureKind,
    span_count: usize,
    error: &str,
    now_ms: u64,
) -> String {
    let preview = truncate_chars(error, ERROR_PREVIEW_CHARS);
    serde_json::json!({
        "ts_ms": now_ms,
        "kind": kind.as_str(),
        "kind_label": kind.label(),
        "spans": span_count,
        "error": preview,
    })
    .to_string()
}

/// 从记录行里取时间戳（解析失败 ⇒ `None`，该行在剪枝时按「过期」处理）。
fn record_ts_ms(line: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()?
        .get("ts_ms")?
        .as_u64()
}

/// 剪枝 + 截断（纯函数）：
/// 1. 丢弃 `now_ms - ts >= ttl_ms` 的过期行；
/// 2. 若仍超过 `max_lines`，只保留最新的 `max_lines` 条。
///
/// `now_ms < ts`（时钟回拨）的行按新鲜处理，不因时钟跳变清空证据。
pub fn prune_records(
    lines: Vec<String>,
    now_ms: u64,
    ttl_ms: u64,
    max_lines: usize,
) -> Vec<String> {
    let mut kept: Vec<String> = lines
        .into_iter()
        .filter(|line| match record_ts_ms(line) {
            Some(ts) => now_ms.saturating_sub(ts) < ttl_ms,
            None => false,
        })
        .collect();
    if kept.len() > max_lines {
        kept.drain(..kept.len() - max_lines);
    }
    kept
}

/// spool 文件路径：与 `sessions.db` 同级的 `otel-spool/export-failures.jsonl`。
///
/// 复用会话库的数据目录解析（含 `PONY_AGENT_DATA_DIR` 旁路与测试隔离），
/// 不另立一套「数据放哪」的规则。
pub fn spool_path() -> PathBuf {
    // 进程内固定：`default_sqlite_path()` 在测试构建下每次返回**唯一**临时路径，
    // 若每次调用都重算，同一进程的记录会散落到不同目录（读回永远为空）。
    // 用 `OnceLock` 锁住一次求值，生产与测试都是「一个进程一个 spool」。
    static SPOOL_PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    SPOOL_PATH
        .get_or_init(|| {
            #[cfg(test)]
            let db = pony_protocol::sqlite_paths::data_dir_override().unwrap_or_else(pony_protocol::sqlite_paths::unique_test_sqlite_path);
            #[cfg(not(test))]
            let db = pony_protocol::sqlite_paths::default_sqlite_path();
            let dir = db
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join("otel-spool");
            dir.join("export-failures.jsonl")
        })
        .clone()
}

/// 追加一条失败记录（有界 + TTL）；返回写入路径，IO 失败返回 `None`。
///
/// 失败**不打扰调用方**：可观测性自身的降级不得再制造一次故障（导出线程本就阻塞在
/// 网络超时上，这里多一次本地写盘的成本可忽略，但异常必须被吞掉并只留一行 warn）。
pub fn record_export_failure(
    kind: ExportFailureKind,
    span_count: usize,
    error: &str,
    now_ms: u64,
) -> Option<PathBuf> {
    let path = spool_path();
    let dir = path.parent()?;
    std::fs::create_dir_all(dir).ok()?;

    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    lines.push(failure_record_line(kind, span_count, error, now_ms));
    let lines = prune_records(lines, now_ms, SPOOL_TTL_MS, SPOOL_MAX_LINES);

    // 原子替换：避免并发/崩溃留下半行 JSON。
    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut file = std::fs::File::create(&tmp).ok()?;
        for line in &lines {
            writeln!(file, "{line}").ok()?;
        }
        file.flush().ok()?;
    }
    std::fs::rename(&tmp, &path).ok()?;
    Some(path)
}

/// B202：导出**放弃**时的完整告警文案（分类 + spool 落盘结果 + 「未上送」措辞）。
///
/// 从 `mod.rs` 的导出循环内联块抽出：文案与落盘口径同住一个文件，且 `mod.rs` 已触文件
/// 规模基线（`npm run size:check`），内联 20 余行会让门禁红。
pub fn give_up(error: &str, attempt: usize, span_count: usize) -> String {
    let max_attempts = super::EXPORT_MAX_ATTEMPTS;
    let kind = classify_export_error(error);
    let spool_tail = match record_export_failure(kind, span_count, error, now_ms()) {
        Some(path) => format!("失败摘要已落本地 spool：{}", path.display()),
        None => "失败摘要落本地 spool 失败（可观测性自身降级，不影响主流程）".to_string(),
    };
    format!(
        "OTLP 导出失败（{}，{attempt}/{max_attempts} 次尝试后放弃，本批 {span_count} 个 span 未上送）：{error}；{spool_tail}",
        kind.label()
    )
}

/// B202：导出**重试前**的告警文案（与 [`give_up`] 同住，避免 `mod.rs` 内联 `format!`）。
///
/// 重试上限与退避时长**不**在本文件另立常量：策略真源仍是 `super`（`mod.rs`）的同名常量，
/// 这里只读不写，避免「文案里的 2 次」与「循环里的 2 次」漂移。
pub fn retry(error: &str, attempt: usize, span_count: usize) -> String {
    let max_attempts = super::EXPORT_MAX_ATTEMPTS;
    let delay_ms = super::EXPORT_RETRY_DELAY_MS;
    format!(
        "OTLP 导出第 {attempt}/{max_attempts} 次尝试失败，{delay_ms}ms 后重试（本批 {span_count} 个 span）：{error}"
    )
}

fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut out: String = text.chars().take(limit).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_default()
    }

    /// 现场错误文本必须判为「出口网络」——这正是 12:20–12:59Z 的实际形态。
    #[test]
    fn b202_timeout_is_classified_as_network() {
        let live = "Operation failed: reqwest::Error { kind: Request, \
                    url: \"https://otel.ponyjob.top/v1/traces\", source: TimedOut }";
        assert_eq!(classify_export_error(live), ExportFailureKind::Network);
        assert_eq!(
            classify_export_error("error sending request: connection reset by peer"),
            ExportFailureKind::Network
        );
    }

    /// 鉴权/配置必须与网络区分，否则会把用户引向错误的排查方向。
    #[test]
    fn b202_auth_and_config_are_distinguished_from_network() {
        assert_eq!(
            classify_export_error("status: 403 Forbidden, invalid token"),
            ExportFailureKind::Auth
        );
        assert_eq!(
            classify_export_error("status: 404 not found, unsupported endpoint"),
            ExportFailureKind::Config
        );
        assert_eq!(
            classify_export_error("something odd happened"),
            ExportFailureKind::Other
        );
    }

    /// 记录行必须自带类别与截断后的错误摘要（不落整段 body）。
    #[test]
    fn b202_record_line_carries_kind_and_truncated_error() {
        let long_error = "x".repeat(600);
        let line = failure_record_line(ExportFailureKind::Network, 4, &long_error, 1_700_000_000_000);
        let parsed: serde_json::Value = serde_json::from_str(&line).expect("必须是合法 JSON");
        assert_eq!(parsed["kind"], "network");
        assert_eq!(parsed["spans"], 4);
        let error = parsed["error"].as_str().unwrap_or_default();
        assert!(error.chars().count() <= ERROR_PREVIEW_CHARS + 1, "错误摘要必须截断");
    }

    /// TTL 到期即剪枝；上限超出只保留最新。
    #[test]
    fn b202_prune_respects_ttl_and_cap() {
        let now = 1_000_000_000_000;
        let fresh = failure_record_line(ExportFailureKind::Network, 1, "e", now - 1_000);
        let expired = failure_record_line(ExportFailureKind::Network, 1, "e", now - SPOOL_TTL_MS - 1);
        let kept = prune_records(vec![expired, fresh.clone()], now, SPOOL_TTL_MS, SPOOL_MAX_LINES);
        assert_eq!(kept, vec![fresh], "过期记录必须被剪掉，新鲜记录保留");

        let many: Vec<String> = (0..5)
            .map(|i| failure_record_line(ExportFailureKind::Network, 1, "e", now - i * 1_000))
            .collect();
        let capped = prune_records(many, now, SPOOL_TTL_MS, 3);
        assert_eq!(capped.len(), 3, "超过上限只保留 max_lines 条");
        assert_eq!(
            capped.last().and_then(|line| record_ts_ms(line)),
            Some(now - 4_000),
            "保留的必须是**最新**记录（最旧先淘汰）"
        );
    }

    /// 解析不出时间戳的坏行不得留在文件里（避免半行 JSON 永久累积）。
    #[test]
    fn b202_broken_lines_are_dropped() {
        let kept = prune_records(
            vec!["not json".to_string()],
            now_ms(),
            SPOOL_TTL_MS,
            SPOOL_MAX_LINES,
        );
        assert!(kept.is_empty());
    }

    /// 端到端：连续记录两批后文件里恰好两行，且落在数据目录的 `otel-spool/` 下。
    #[test]
    fn b202_spool_appends_bounded_records() {
        let path = spool_path();
        assert!(
            path.ends_with("otel-spool/export-failures.jsonl"),
            "spool 必须落在数据目录的 otel-spool/ 下，实际 = {}",
            path.display()
        );
        // 测试隔离：默认路径在 cfg(test) 下已是进程唯一临时目录，清掉历史残留即可。
        let _ = std::fs::remove_file(&path);

        let now = now_ms();
        assert!(record_export_failure(ExportFailureKind::Network, 2, "TimedOut", now).is_some());
        assert!(record_export_failure(ExportFailureKind::Config, 1, "404", now).is_some());

        let content = std::fs::read_to_string(&path).expect("spool 文件必须存在");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "两批失败 ⇒ 两行记录");
        assert!(lines[0].contains("\"network\""));
        assert!(lines[1].contains("\"config\""));
    }
}
