use std::cell::RefCell;
use std::time::{Duration, Instant};

use crate::agent::error_code::ErrorCode;

/// Backoff configuration for retry policies.
#[derive(Debug, Clone, Copy)]
pub struct BackoffConfig {
    pub initial_delay_ms: u64,
    pub multiplier: f64,
    pub max_delay_ms: u64,
    pub max_retries: u32,
    pub total_budget_ms: u64,
    pub jitter_kind: JitterKind,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 400,
            multiplier: 2.0,
            max_delay_ms: 5_000,
            max_retries: 3,
            total_budget_ms: 12_000,
            jitter_kind: JitterKind::Full,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitterKind {
    None,
    Equal,
    Full,
}

/// Pure delay planner: given attempt index (1-based), returns the delay before that attempt.
/// attempt=0 (first call) returns Duration::ZERO.
pub fn compute_delay(attempt: u32, config: &BackoffConfig, jitter_sample: f64) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    let base = config.initial_delay_ms as f64 * config.multiplier.powi(attempt as i32 - 1);
    let clamped = base.min(config.max_delay_ms as f64);
    let delay = match config.jitter_kind {
        JitterKind::None => clamped,
        JitterKind::Equal => clamped * (0.5 + jitter_sample * 0.5),
        JitterKind::Full => jitter_sample * clamped,
    };
    Duration::from_millis(delay as u64)
}

/// Retry decision for the escalation contract.
#[derive(Debug, Clone, PartialEq)]
pub enum RetryDecision {
    Retry {
        delay_ms: u64,
    },
    Fallback {
        target: FallbackTarget,
        reason: String,
    },
    Escalate {
        reason: String,
    },
    Abort {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum FallbackTarget {
    Sync,
    LocalSynthesis,
}

/// Stream output commitment state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    PreConnection,
    NoDelta,
    ReasoningOnly,
    VisibleTextStarted,
    ToolCallStarted,
}

impl StreamState {
    pub fn may_auto_retry(self) -> bool {
        matches!(
            self,
            Self::PreConnection | Self::NoDelta | Self::ReasoningOnly
        )
    }

    pub fn may_stream_to_sync_fallback(self) -> bool {
        matches!(self, Self::NoDelta | Self::ReasoningOnly)
    }

    pub fn transition(self, kind: StreamEventKind) -> Self {
        if self == Self::ToolCallStarted {
            return Self::ToolCallStarted;
        }
        if matches!(kind, StreamEventKind::Text) {
            return Self::VisibleTextStarted;
        }
        if matches!(kind, StreamEventKind::MixedReasoningText) {
            return Self::VisibleTextStarted;
        }
        match kind {
            StreamEventKind::Reasoning if self == Self::NoDelta || self == Self::PreConnection => {
                Self::ReasoningOnly
            }
            StreamEventKind::ToolCall => Self::ToolCallStarted,
            StreamEventKind::ConnectionEstablished if self == Self::PreConnection => Self::NoDelta,
            _ => self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEventKind {
    ConnectionEstablished,
    Reasoning,
    Text,
    MixedReasoningText,
    ToolCall,
}

/// Provider failure classification.
#[derive(Debug, Clone, PartialEq)]
pub enum FailureKind {
    TransientRetryable { details: String },
    NonRetryable { details: String },
    RequiresRequestMutation { details: String },
    UnsafeToRetry { details: String },
}

impl FailureKind {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::TransientRetryable { .. })
    }
}

/// Marker type for retry cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

/// Sleeper trait for injectable sleep in tests.
/// Returns `Err(Cancelled)` if the sleep was interrupted.
pub trait Sleeper: Send + Sync {
    fn sleep(
        &self,
        duration: Duration,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), Cancelled>;
}

pub struct StdThreadSleeper;

impl Sleeper for StdThreadSleeper {
    fn sleep(
        &self,
        duration: Duration,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), Cancelled> {
        let start = std::time::Instant::now();
        while start.elapsed() < duration {
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(Cancelled);
            }
            std::thread::sleep(std::cmp::min(
                std::time::Duration::from_millis(50),
                duration.saturating_sub(start.elapsed()),
            ));
        }
        Ok(())
    }
}

pub struct FakeSleeper {
    pub total_slept: std::sync::atomic::AtomicU64,
}

impl FakeSleeper {
    pub fn new() -> Self {
        Self {
            total_slept: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl Sleeper for FakeSleeper {
    fn sleep(
        &self,
        duration: Duration,
        _cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), Cancelled> {
        self.total_slept.fetch_add(
            duration.as_millis() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        Ok(())
    }
}

/// Budget tracker for request-level retry.
#[derive(Debug, Clone)]
pub struct RetryBudget {
    pub max_retries: u32,
    pub total_budget_ms: u64,
    pub elapsed_ms: u64,
    pub attempts_used: u32,
}

impl RetryBudget {
    pub fn new(max_retries: u32, total_budget_ms: u64) -> Self {
        Self {
            max_retries,
            total_budget_ms,
            elapsed_ms: 0,
            attempts_used: 0,
        }
    }

    pub fn is_exhausted(&self) -> bool {
        self.attempts_used >= self.max_retries || self.elapsed_ms >= self.total_budget_ms
    }

    pub fn exhausted_reason(&self) -> Option<&'static str> {
        if self.is_exhausted() {
            if self.attempts_used >= self.max_retries {
                Some("attempt_limit_exhausted")
            } else {
                Some("time_budget_exhausted")
            }
        } else {
            None
        }
    }

    pub fn record_attempt(&mut self, delay_ms: u64) {
        self.attempts_used += 1;
        self.elapsed_ms = self.elapsed_ms.saturating_add(delay_ms);
    }

    pub fn record_execution(&mut self, execution_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(execution_ms);
    }
}

/// T2-B：单跳重试观测报告（只读观测，不参与调度）。
///
/// 口径（与 `FakeSleeper` 只计 sleep 的口径一致）：
/// - `attempts_total` = `operation` 被调用次数（含最终成功的那一次）；
/// - `slept_ms` = sleep-only 累计（成功执行的 sleep 之和，不含请求自身耗时）；
/// - `switched_to_rl` = 本跳内是否切换过 rate-limit 长退避配置；
/// - `scope` = 重试作用域（"decision" / "followup"）；
/// - `exhausted_reason` = 终态 `budget.exhausted_reason()`（未耗尽为 `None`，
///   含成功返回的情况）。
#[derive(Debug, Clone)]
pub struct SingleHopRetryReport {
    pub attempts_total: u32,
    pub slept_ms: u64,
    pub switched_to_rl: bool,
    pub scope: &'static str,
    pub exhausted_reason: Option<&'static str>,
}

/// Retry-After header handling.
#[derive(Debug, Clone)]
pub struct RetryAfter {
    pub seconds: Option<u64>,
}

impl RetryAfter {
    pub fn parse(header_value: &str) -> Self {
        if let Ok(secs) = header_value.trim().parse::<u64>() {
            return Self {
                seconds: Some(secs),
            };
        }
        // HTTP-date format parsing is deferred to a follow-up.
        // The seconds format covers the majority of Retry-After usage (429/503).
        Self { seconds: None }
    }

    pub fn effective_delay(&self, backoff_delay_ms: u64, remaining_budget_ms: u64) -> Option<u64> {
        let ra = self.seconds?;
        let ra_ms = ra.saturating_mul(1000);
        if ra_ms == 0 || ra_ms > remaining_budget_ms {
            return None;
        }
        Some(backoff_delay_ms.max(ra_ms).min(remaining_budget_ms))
    }
}

/// Stage2（task-5 R1/R2.4/R3.1）：关键词表收编为注册表单一来源。
/// 七张码表常量即 R1 八分组①–⑥ + R2.4 七词的唯一真源；`classify` / `is_rate_limit_error`
/// 只消费这些常量做 `contains` 判定，不再内联字面量。全仓 `lower.contains` /
/// `to_ascii_lowercase` 零新增：本文件内 lowering 仍仅两处（classify / is_rate_limit_error）。
///
/// 分组→注册表映射（R1）：
/// ① timeout 四词 → [`ErrorCode::UpstreamTimeout`]；② 429 三词 → `UpstreamRateLimited`；
/// ③ 5xx/408 四码 → `UpstreamServerError`；③′ 边缘 5xx（520–525/527/530，
/// 实证驱动，ADR-0086；525 于 2026-11-12 纳入）→ 同上；④ 连接三词 → `UpstreamNetwork`
/// （禁扩 `connect` 泛词）；④′ 传输层标志 `type=connect|request` → `UpstreamNetwork`
/// （无状态码，置于所有状态码判据之后）；⑤ 400 系七码 → `UpstreamBadRequest`/`UpstreamAuth`
/// （401 与 400 不区分冻结，同落 NonRetryable）；⑥ context 四词 → `UpstreamContextOverflow`；
/// ⑦ 其余默认 Abort；⑧ decide/budget/RetryAfter/StreamState 语义冻结（本 Stage 不动）。
///
/// R2.4 七词（`is_rate_limit_error`，PA-171 增补后为九词）：429/rate limit/rate_limit/
/// ratelimit/too many requests/tpm/quota + usage limit/allowance → 限流类（冷却）候选。
/// PA-171 C3 反转 PA-100 的 quota-only 残余：quota-only 措辞（无 429 特征）现经
/// [`UPSTREAM_QUOTA_HINTS`] 落 [`ErrorCode::UpstreamQuota`] ⇒ `classify` 判 NonRetryable
/// 立即 Abort（不再打 5 次短退避），由 provider 侧登记冷却窗口；429+quota 混合报文仍先命中
/// 限流表（`UpstreamRateLimited`），按 `Retry-After`/恢复时间冷却。
pub(crate) const UPSTREAM_TIMEOUT_HINTS: &[&str] = &[
    "type=timeout",
    "timeout",
    "timed out",
    "deadline has elapsed",
];
pub(crate) const UPSTREAM_RATE_LIMITED_HINTS: &[&str] =
    &["429", "rate limit", "rate_limit", "too many requests", "tpm"];
/// PA-171 C3：quota-only 措辞（**不含** 429/rate-limit 特征）→ [`ErrorCode::UpstreamQuota`]。
/// 判定置于限流表**之后**：`429 + quota` 混合报文仍按限流处理（既有语义不变），
/// 纯配额/用量上限/额度耗尽文案则走"立即 Abort + 冷却"，不再白烧 5 次短退避。
pub(crate) const UPSTREAM_QUOTA_HINTS: &[&str] = &[
    "quota",
    "exceeded_current_quota_error",
    "usage limit",
    "allowance",
    "billing window",
];
pub(crate) const UPSTREAM_SERVER_ERROR_HINTS: &[&str] = &["502", "503", "504", "408"];
/// ③′ 边缘 5xx（ADR-0086）：语义与 502/503/504 同为「暂时性上游不可用」，但码值
/// 不在 RFC 标准表内（reqwest 显示为 `<unknown status code>`），原码表一条都不命中
/// ⇒ 一律 `NonRetryable` 直接 Abort，退避预算从未启动。
///
/// 525 **纳入**（ADR `.agents/notes/implemented/bug-fix/2026-11-12-upstream-525-retry-classification.md`）：
/// 原判定沿用「源站 TLS 配置错误」的静态理由排除，但实证（会话 `pa_01a1cf14` 3 轮连撞
/// `HTTP 525` @6–8s，跨 gemini / hy4free 两个模型）显示 Cloudflare 525 是**边缘与源站握手
/// 瞬时失败**（源站重启/负载抖动/握手超时），与 522 同属「边缘没能完成交换」，秒级可自愈；
/// 静态排除导致 5 次预算从未启动、也不换手，同一网关被连撞三次。故与 ③′ 其余码同组。
///
/// 526 仍**排除**：`Invalid SSL Certificate` 是源站证书无效/过期/不受信，属确定性配置错误，
/// 重试与换到同源站的其他模型都不会改变结果，保持 Abort（不为让退避「看起来有效」而吞掉配置故障）。
/// 500 亦维持既有冻结（NonRetryable，见 `stage2_upstream_code_for_hints_covers_r1_groups`）。
pub(crate) const UPSTREAM_EDGE_SERVER_ERROR_HINTS: &[&str] =
    &["520", "521", "522", "523", "524", "525", "527", "530"];
pub(crate) const UPSTREAM_NETWORK_HINTS: &[&str] =
    &["connection reset", "connection refused", "dns"];
/// ④′ 传输层阶段标志（ADR-0086）：`type=connect` / `type=request` /
/// `type=connect+request`。这类失败**没有 HTTP 状态码**（Connect/SendRequest 阶段即终止），
/// 码表按状态码与措辞匹配全部落空 ⇒ 直接 Abort。判据用结构化标志而非裸 `connect`
/// 泛词（`connect` 单独出现仍不命中，沿用 R1④ 禁扩约束）。
pub(crate) const UPSTREAM_TRANSPORT_HINTS: &[&str] = &["type=connect", "type=request"];
pub(crate) const UPSTREAM_BAD_REQUEST_HINTS: &[&str] =
    &["400", "401", "403", "404", "422", "407", "413"];
pub(crate) const UPSTREAM_CONTEXT_OVERFLOW_HINTS: &[&str] = &[
    "context too large",
    "context_length",
    "max_tokens",
    "payload too large",
];
pub(crate) const UPSTREAM_AUTH_HINTS: &[&str] = &["401", "403"];
/// PA-171 C3-S1：`classify_error_class` 的 auth 措辞补充（与 `UPSTREAM_AUTH_HINTS` 同一
/// 注册表出口；仅供审计/文案分类消费，不改 `classify` 的重试结论）。
/// L3-B 复审追加 `unauthorized`/`forbidden`/`authentication`（词边界判定，禁止裸 contains）。
pub(crate) const UPSTREAM_AUTH_WORDING_HINTS: &[&str] = &[
    "api key",
    "invalid_api_key",
    "unauthorized",
    "forbidden",
    "authentication",
];
/// PA-171 C3-S1：`classify_error_class` 的 bad_request 措辞补充（同上，单一注册表）。
pub(crate) const UPSTREAM_BAD_REQUEST_WORDING_HINTS: &[&str] = &["invalid request", "missing field"];
pub(crate) const RATE_LIMIT_LONG_BACKOFF_HINTS: &[&str] = &[
    "429",
    "rate limit",
    "rate_limit",
    "ratelimit",
    "too many requests",
    "tpm",
    "quota",
    // PA-171 C3：两条真实可达的限流/配额措辞（诊断报文
    // `You've reached your 5-hour usage limit` / `out of its allowance`），
    // 历史漏判 ⇒ 走 500ms→8s 短退避打满后 Abort。
    "usage limit",
    "allowance",
];

/// wave-35/C3(生产判据修复):分类只允许看**原因文本**,不得看 URL/endpoint 片段。
///
/// 缺陷:provider 把 `响应预览: {endpoint}`(`provider/openai_sse.rs`,错误文本里带
/// `http://127.0.0.1:<随机端口>/v1`)与 `endpoint={url}` 拼进 message,而码表用裸
/// `contains` 在整段 message 上匹配 ⇒ 端口含 429/502/503/504/408 时(如 `:42953`)
/// 把「永久性协议错误」误判为 `TransientRetryable` ⇒ 5 次重试耗尽 mock 队列 ⇒
/// 回合相位错误(`control_plane` 相位断言 flake,p≈2.8%);endpoint 里的措辞
/// (如 `timeout-gw.internal`)同样会污染分组。
///
/// 修法:匹配前只从 `://` 分隔符剥到 URL 终点,其前文本**整体保留**(`endpoint=` 标签、
/// scheme 名、紧贴 URL 的数字与多字节字符都留下)。诊断展示不受影响 —— `FailureKind`
/// 的 `details` 仍是原始 message。
///
/// 切片安全(波次 G F1):所有切片边界只来自 `str::find`,天然是 char 边界;
/// **禁止**对 `find`/`rfind` 返回的字节索引做 `+1`/`-1` 偏移(会把边界切进多字节字符内部,
/// 如 `访问https://x/v1` 里 `问` 的 `index+1` ⇒ `byte index 4 is not a char boundary` panic,
/// 且入参含上游可控文本)。
///
/// 已知取舍:`:` 不可作为 URL 终点(端口本身即 `host:port`),因此紧贴 URL、不带
/// 空白/分隔符的尾部原因码会被一并剥离;生产错误文本一律以 `; ` / 空白分隔。
///
/// PA-144:除带 scheme 的 `scheme://host:port/path` 外,还要剥离 **scheme-less**
/// `host:port[/path]`(如 `endpoint=127.0.0.1:42953/v1`、`endpoint=localhost:50208`)——
/// 生产可达路径是 provider 把 endpoint 拼进错误正文,而 `config.rs` 的 base_url 解析
/// 不校验 scheme,于是端口数字(乃至恰为码值的 `:429`/`:502`)会污染分类。
fn strip_endpoint_fragments(lower: &str) -> String {
    let mut out = String::with_capacity(lower.len());
    let mut rest = lower;
    while let Some(scheme_sep) = rest.find("://") {
        // URL 之前的文本整体保留(含紧贴的数字,如 `HTTP 502https://…` 的 502;
        // 波次 G F2:旧实现在此回退吞掉 scheme 名 ⇒ URL 前紧贴的数字一并丢失)。
        // PA-143:紧贴 `://` 的 **scheme 名字母串**(`https` 等)剥掉 —— 数字与字母粘连时
        // 词边界判据会把 `429https`/`502https` 当成一个 token 拒绝,而 `429abc` 这类
        // 非 scheme 粘连仍保持不命中;只剥 ASCII 字母,数字原样保留参与分类。
        let prefix = &rest[..scheme_sep];
        let mut prefix_end = prefix.len();
        while prefix_end > 0 && prefix.as_bytes()[prefix_end - 1].is_ascii_alphabetic() {
            prefix_end -= 1;
        }
        out.push_str(&prefix[..prefix_end]);
        // URL 终点:空白或常见分隔符(引号/分号/逗号/括号)。
        let tail = &rest[scheme_sep..];
        let url_end = tail
            .find(|c: char| {
                c.is_whitespace()
                    || matches!(c, '"' | '\'' | ';' | ',' | ')' | ']' | '}' | '<' | '>' | '`')
            })
            .unwrap_or(tail.len());
        rest = &tail[url_end..];
    }
    out.push_str(rest);
    // PA-144:再剥离 scheme-less 的 host:port[/path]。
    strip_schemeless_host_port(&out)
}

/// PA-144:剥离没有 scheme 的 `host:port[/path]` 片段,避免端口数字参与码表判定。
///
/// 只剥离**可确认是网络端点**的命中,避免吞掉正文语义(C-144「不得过度剥离」):
/// - host 必须形如域名/IP(`localhost`、含 `.` 的主机名)或**紧跟 `=`**(`endpoint=`/
///   `host=` 等标签)⇒ `status:502`、时间 `12:30`、端口范围 `8000:8010` 不被剥离,
///   `status:502` 这类真实原因码仍参与分类;
/// - 端口必须是 ASCII 数字,且其后只能是串尾/分隔符/`/`(路径);`40832ms` 这类
///   数字后接字母的命中不剥离;
/// - 路径从 `/` 剥到空白或 shell/JSON 分隔符为止。
///
/// 字节安全:**所有**切片边界只落在 ASCII `:`、ASCII 数字/路径字符、ASCII 分隔符或
/// 串尾上,天然是 char 边界;反向扫描遇多字节字符(首字节非 ASCII)即停止,不会切进
/// 多字节字符内部。
fn strip_schemeless_host_port(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b':' {
            i += 1;
            continue;
        }
        // 端口:ASCII 数字串(至少一位)。
        let mut port_end = i + 1;
        while port_end < bytes.len() && bytes[port_end].is_ascii_digit() {
            port_end += 1;
        }
        if port_end == i + 1 {
            i += 1;
            continue;
        }
        // host:反向吃 `[a-z0-9.-]`,遇非 ASCII(多字节字符首字节)或其它符号即停。
        // `]` 与 `[` 参与 IPv6 literal 形态的回扫(如 `[::1]:port`),`=` 参与
        // `endpoint=` 标签形态;其余非 ASCII 字节即停,不切进多字节字符。
        let mut host_start = i;
        while host_start > 0 {
            let b = bytes[host_start - 1];
            if b.is_ascii_alphanumeric()
                || b == b'.'
                || b == b'-'
                || b == b':'
                || b == b'['
                || b == b']'
            {
                host_start -= 1;
            } else {
                break;
            }
        }
        // IPv6 literal:回扫吃到 `[` 且段内含 `:`(压缩 `::` 与完整形式都由 `:` 覆盖)
        // ⇒ 按 IPv6 端点处理;`[]` 内段不得为空且不得以 `:` 结尾误吞(如 `[::1]`)。
        let mut ipv6_literal = false;
        if host_start > 0 && bytes[host_start - 1] == b'[' {
            let inner = &text[host_start..i];
            // 期望形如 `...]:port` —— 回扫从 port 前的 `:` 起,段首应是 `]` 之后的空段,
            // 实际 inner 已包含 `]` 前的地址部分;此时检查 inner 是否含 `]` 与 `:`。
            if let Some(bracket_end) = inner.rfind(']') {
                // literal 形态:回扫段以 `]` 结尾(addr = `[...`),且地址非空含 `:`。
                if bracket_end + 1 == inner.len() {
                    let addr = &inner[..bracket_end];
                    if !addr.is_empty() && addr.starts_with('[') && addr.contains(':') {
                        host_start -= 1;
                        ipv6_literal = true;
                    }
                }
            }
        }
        let host = &text[host_start..i];
        // IPv6 literal 形态要求 host 段形如 `xxx]`(地址已在回扫中含 `[`..`]`),
        // 即回扫停在 `:`(端口分隔符)时上一字符应为 `]`;否则是 `[:` 这类
        // literal 内部的伪冒号,不得按端点剥离(C-144:不得过度剥离/误剥)。
        let preceded_by_equals = !ipv6_literal
            && host_start > 0
            && bytes[host_start - 1] == b'='
            && !host.contains('[')
            && !host.contains(']');
        let host_looks_like_endpoint = !host.is_empty()
            && !host.starts_with('.')
            && !host.ends_with('.')
            && !host.ends_with('-')
            && (ipv6_literal
                || host.ends_with(']')
                || host.contains('.')
                || host == "localhost"
                || preceded_by_equals);
        if !host_looks_like_endpoint {
            i += 1;
            continue;
        }
        // 路径:紧邻的 `/...` 一并剥离(端口与路径都不参与分类)。
        let mut span_end = port_end;
        if span_end < bytes.len() && bytes[span_end] == b'/' {
            span_end += 1;
            while span_end < bytes.len() {
                let b = bytes[span_end];
                if b.is_ascii_whitespace()
                    || matches!(
                        b,
                        b'"' | b'\'' | b';' | b',' | b')' | b']' | b'}' | b'<' | b'>' | b'`'
                    )
                {
                    break;
                }
                span_end += 1;
            }
        }
        out.push_str(&text[copied..host_start]);
        copied = span_end;
        i = span_end;
    }
    out.push_str(&text[copied..]);
    out
}

/// 码表命中判定：**不得**是整段文本上的裸子串（PA-143）。
///
/// 缺陷：`elapsed=40832ms` 内含 `408`、JSON 错误码 `"429001"` 内含 `429`、
/// `"1502"` 内含 `502`，裸 `contains` 会把这些与原因无关的数字片段当成上游码 ⇒
/// 重试结论取决于报文里恰好出现了哪些数字。
///
/// 判据（词边界/结构化）：
/// - 数字码（如 `429`/`502`/`408`）：命中位置前后**不得紧邻 ASCII 数字**；
/// - 措辞（如 `timed out`/`connection reset`/`rate limit`）：命中串首尾为字母数字时，
///   不得与相邻字母数字粘连；
/// - 真实原因码 `HTTP 429/502/503/504/408`、`status:502`、`operation timed out`、
///   `connection reset` 仍命中（C-143-4 护栏）。
fn hints_match(lower: &str, hints: &[&str]) -> bool {
    hints.iter().any(|hint| hint_matches(lower, hint))
}

fn hint_matches(lower: &str, hint: &str) -> bool {
    for (start, matched) in lower.match_indices(hint) {
        // 数字码:前后不得紧邻字母数字(挡住 `1502` 里的 `502`、`429001` 里的 `429`、
        // `429abc`/`abc429`/`error_429x` 等污染片段;独立 `429` 仍命中,PA-143)。
        // 按字符判定(char 边界安全),多字节字母数字同样粘住。
        if lower[..start].chars().next_back().is_some_and(is_word_char) {
            continue;
        }
        let end = start + matched.len();
        if lower[end..].chars().next().is_some_and(is_word_char) {
            continue;
        }
        return true;
    }
    false
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// 码表分组 → 注册表码（R1 映射的唯一出口；classify 经此归一后再定 FailureKind）。
/// 返回 `None` = ⑦其余默认（NonRetryable Abort）。
pub(crate) fn upstream_code_for_hints(lower: &str) -> Option<ErrorCode> {
    // wave-35/C3: 先剥离 endpoint/URL 片段,再走码表(端口数字不得决定分组)。
    let stripped = strip_endpoint_fragments(lower);
    let lower = stripped.as_str();
    if hints_match(lower, UPSTREAM_TIMEOUT_HINTS) {
        return Some(ErrorCode::UpstreamTimeout);
    }
    if hints_match(lower, UPSTREAM_RATE_LIMITED_HINTS) {
        return Some(ErrorCode::UpstreamRateLimited);
    }
    // PA-171 C3：quota-only（配额/用量上限/额度耗尽）→ UpstreamQuota。
    // 置于限流判据之后、5xx 判据之前：429+quota 混合报文仍先落 UpstreamRateLimited。
    if hints_match(lower, UPSTREAM_QUOTA_HINTS) {
        return Some(ErrorCode::UpstreamQuota);
    }
    if hints_match(lower, UPSTREAM_SERVER_ERROR_HINTS) {
        return Some(ErrorCode::UpstreamServerError);
    }
    if hints_match(lower, UPSTREAM_EDGE_SERVER_ERROR_HINTS) {
        return Some(ErrorCode::UpstreamServerError);
    }
    if hints_match(lower, UPSTREAM_NETWORK_HINTS) {
        return Some(ErrorCode::UpstreamNetwork);
    }
    if hints_match(lower, UPSTREAM_BAD_REQUEST_HINTS) {
        // 401 与 400 不区分冻结：同落 NonRetryable（R1⑤）。
        if hints_match(lower, UPSTREAM_AUTH_HINTS) {
            return Some(ErrorCode::UpstreamAuth);
        }
        return Some(ErrorCode::UpstreamBadRequest);
    }
    if hints_match(lower, UPSTREAM_CONTEXT_OVERFLOW_HINTS) {
        return Some(ErrorCode::UpstreamContextOverflow);
    }
    // ④′ 传输层标志置于**所有状态码判据之后**（ADR-0086）：报文同时含真实 4xx 时
    // 仍按状态码落 NonRetryable，避免把「服务端明确拒绝」误升为可重试。
    if hints_match(lower, UPSTREAM_TRANSPORT_HINTS) {
        return Some(ErrorCode::UpstreamNetwork);
    }
    None
}

/// Provider-specific retry policy combining config, classification, and budget.
pub struct ProviderRetryPolicy {
    pub config: BackoffConfig,
    pub sleeper: Box<dyn Sleeper>,
}

impl ProviderRetryPolicy {
    pub fn new(config: BackoffConfig) -> Self {
        Self {
            config,
            sleeper: Box::new(StdThreadSleeper),
        }
    }

    pub fn with_sleeper(config: BackoffConfig, sleeper: Box<dyn Sleeper>) -> Self {
        Self { config, sleeper }
    }

    /// Classify a provider error based on its message content.
    /// This does NOT consider stream state; stream safety is handled by `decide`.
    ///
    /// Stage2 遗留桥：判定语义与 R1 八分组逐项一致，仅关键词来源收编为上方码表常量。
    pub fn classify(&self, err: &str) -> FailureKind {
        let lower = err.to_ascii_lowercase();
        match upstream_code_for_hints(&lower) {
            // ①–④ → TransientRetryable（R1①②③④）。
            Some(
                ErrorCode::UpstreamTimeout
                | ErrorCode::UpstreamRateLimited
                | ErrorCode::UpstreamServerError
                | ErrorCode::UpstreamNetwork,
            ) => FailureKind::TransientRetryable {
                details: err.to_string(),
            },
            // ⑥ → RequiresRequestMutation（R1⑥）。
            Some(ErrorCode::UpstreamContextOverflow) => FailureKind::RequiresRequestMutation {
                details: err.to_string(),
            },
            // ⑤⑦ → NonRetryable（R1⑤⑦；401/403 同落此分支，不区分冻结）。
            // PA-171 C3：quota-only（UpstreamQuota）同样 NonRetryable ⇒ 立即 Abort，
            // 由 provider 侧按恢复窗口登记冷却（不再 5 次短退避打满）。
            Some(ErrorCode::UpstreamAuth)
            | Some(ErrorCode::UpstreamBadRequest)
            | Some(ErrorCode::UpstreamQuota)
            | None => FailureKind::NonRetryable {
                details: err.to_string(),
            },
            // 码表唯一出口仅产出上述七码；兜底永不可达（穷尽即契约）。
            Some(other) => FailureKind::NonRetryable {
                details: format!("{}: {}", other.as_str(), err),
            },
        }
    }

    /// Decide what to do with a failure given stream state and budget.
    /// Stream safety is checked here: states with visible output or tool-call commitment
    /// prohibit auto-retry and produce Abort.
    /// Fallback to sync mode should be decided at a higher orchestrator level.
    pub fn decide(
        &self,
        failure: &FailureKind,
        budget: &RetryBudget,
        attempt: u32,
        retry_after: Option<&RetryAfter>,
        stream_state: StreamState,
    ) -> RetryDecision {
        if !stream_state.may_auto_retry() {
            return RetryDecision::Abort {
                reason: format!(
                    "stream state {:?} prohibits auto-retry: {}",
                    stream_state,
                    failure.details()
                ),
            };
        }
        if !failure.is_retryable() {
            return RetryDecision::Abort {
                reason: format!("{:?}: {}", failure, failure.details()),
            };
        }
        if budget.is_exhausted() {
            return RetryDecision::Escalate {
                reason: format!("budget exhausted: {:?}", budget.exhausted_reason()),
            };
        }
        let base_delay = compute_delay(attempt, &self.config, 0.5);
        let delay = if let Some(ra) = retry_after {
            let remaining = self
                .config
                .total_budget_ms
                .saturating_sub(budget.elapsed_ms);
            ra.effective_delay(base_delay.as_millis() as u64, remaining)
                .unwrap_or(base_delay.as_millis() as u64)
        } else {
            base_delay.as_millis() as u64
        };
        RetryDecision::Retry {
            delay_ms: delay.min(self.config.max_delay_ms),
        }
    }

    /// Check if fallback to sync mode is appropriate for this failure+stream combination.
    /// Stream→sync fallback is safe when the stream has not yet committed visible text
    /// (NoDelta or ReasoningOnly) AND the error is non-retryable.
    /// This is a higher-level policy decision, not part of the auto-retry loop.
    pub fn should_fallback_to_sync(
        &self,
        failure: &FailureKind,
        stream_state: StreamState,
    ) -> bool {
        !failure.is_retryable() && stream_state.may_stream_to_sync_fallback()
    }
}

impl FailureKind {
    pub fn details(&self) -> &str {
        match self {
            Self::TransientRetryable { details }
            | Self::NonRetryable { details }
            | Self::RequiresRequestMutation { details }
            | Self::UnsafeToRetry { details } => details,
        }
    }

    pub fn is_unsafe_to_retry(&self) -> bool {
        matches!(self, Self::UnsafeToRetry { .. })
    }

    pub fn requires_request_mutation(&self) -> bool {
        matches!(self, Self::RequiresRequestMutation { .. })
    }
}

/// PA-100/PA-171：rate-limit 特征判定（单一来源）。`classify` 仍把这类错误统一归为
/// `TransientRetryable`（可重试）；本函数供重试调度方/文案层进一步区分"应使用冷却窗口
/// 覆盖分钟级限流"的子类，避免在调用侧各自维护关键词列表造成漂移。
///
/// 消费前提（PA-171 C3 反转 PA-100 残余）：
/// - quota-only（报文仅含 quota/usage limit/allowance 等措辞，不含 "429"/"rate limit"）：
///   `classify` 落 [`ErrorCode::UpstreamQuota`] ⇒ NonRetryable 立即 Abort；provider 侧按
///   本判定登记冷却窗口，本判定同时是 TUI 文案的"限流/配额"单一判据；
/// - quota+429 混合（报文同时含 quota 与 "429"/"rate limit" 特征）：
///   `classify` 先判 TransientRetryable，调度方再按 `Retry-After`/恢复时间决定：
///   窗口超出本次预算 ⇒ 立即失败（冷却），否则沿用既有 RL 长退避；
///
/// 判定优先于措辞细节：报文同时含 timeout 与 rate-limit 特征时（如网关把上游
/// 429 包装成超时文案），按 rate-limit 处理——冷却对两类瞬时故障都安全，
/// 反向（短退避撞限流窗口）则必然失败。
pub fn is_rate_limit_error(err: &str) -> bool {
    // R2.4 九词（PA-171 增补 usage limit / allowance）：唯一来源 RATE_LIMIT_LONG_BACKOFF_HINTS。
    let lower = err.to_ascii_lowercase();
    // wave-35/C3: same as classify -- endpoint/URL fragments (ports can contain 429)
    // must not drive the decision. R2.4 wording table itself is unchanged.
    hints_match(&strip_endpoint_fragments(&lower), RATE_LIMIT_LONG_BACKOFF_HINTS)
}

// ── PA-171 C3/C4/C5：限流冷却窗口与失败归因（单一出口，调用方只消费此处） ──────────────

/// C3-S4：`Retry-After` 在错误文本里的唯一结构化后缀（两段解析同源，禁止第二处拼装）。
pub const RETRY_AFTER_MARKER: &str = "retry_after=";

/// C3-S4：把 `Retry-After` 秒数追加为结构化后缀（幂等：已有标记则不重复追加）。
///
/// L3-B：标记必须落在**头部段**（`；响应正文` 之前）——正文是上游可控内容，
/// 不得让正文里的 `retry_after=999999` 覆盖真实头（解析侧只读头部段）。
pub fn append_retry_after_marker(err: &str, seconds: u64) -> String {
    // L3-B：幂等只看**头部段**——正文里同名 token 不得短路权威头值的写入。
    if parse_retry_after_marker(err).is_some() {
        return err.to_string();
    }
    match err.find("响应正文") {
        Some(index) => {
            let (head, body) = err.split_at(index);
            format!(
                "{}；{RETRY_AFTER_MARKER}{seconds}；{body}",
                head.trim_end_matches('；')
            )
        }
        None => format!("{err}；{RETRY_AFTER_MARKER}{seconds}"),
    }
}

/// C3-S4：从错误文本解析 `Retry-After` 秒数（唯一解析点）。
///
/// L3-B：只解析**头部段**（`；响应正文` 之前）——正文段不参与，避免正文劫持冷却窗口。
pub fn parse_retry_after_marker(err: &str) -> Option<u64> {
    let head_end = err.find("响应正文").unwrap_or(err.len());
    let head = &err[..head_end];
    let index = head.find(RETRY_AFTER_MARKER)?;
    let digits: String = head[index + RETRY_AFTER_MARKER.len()..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse::<u64>().ok()
}

/// L3-B：错误文本的头部段（`；响应正文` 之前）——结构化判定只看这一段。
pub fn error_head_segment(error: &str) -> &str {
    let head_end = error.find("响应正文").unwrap_or(error.len());
    &error[..head_end]
}

/// C5 文案层 auth 判定：只看头部段 + 词边界（禁止裸 `contains` 扫全文）。
///
/// L3-B BLOCKING 反例：`429 … 耗时=4031ms` / `429 … 耗时=401ms + quota` / `429 … 正文含
/// "api key"` 均**不得**判为 auth。状态码 `401/403` 走 [`hints_match`] 词边界判据
/// （`4031`、`401ms` 不命中），响应正文整体不参与。
pub fn is_auth_error(error: &str) -> bool {
    let head = error_head_segment(error).to_ascii_lowercase();
    let stripped = strip_endpoint_fragments(&head);
    hints_match(&stripped, UPSTREAM_AUTH_HINTS)
        || hints_match(&stripped, UPSTREAM_AUTH_WORDING_HINTS)
}

/// L3-B ADVISORY：provider 已在冷却期的结构化标记（HTTP 出口登记冷却 / 冷却短路时追加）。
///
/// 重试编排见此标记立即 Abort（**全 scope**：Decision 与 Followup 都不 sleep、不再发 HTTP），
/// 这是「冷却期 0 次 HTTP」在编排层的唯一判据（不依赖错误分类，避免把正常限流重试误伤）。
pub const COOLDOWN_ACTIVE_MARKER: &str = "provider_cooldown_active=";

/// L3-B ADVISORY：给错误文本追加冷却标记（幂等）。
pub fn append_cooldown_active_marker(err: &str) -> String {
    if err.contains(COOLDOWN_ACTIVE_MARKER) {
        return err.to_string();
    }
    format!("{err}；{COOLDOWN_ACTIVE_MARKER}true")
}

/// L3-B ADVISORY：该错误是否表示「provider 已进入/处于冷却期」。
pub fn is_cooldown_active_error(err: &str) -> bool {
    err.contains(COOLDOWN_ACTIVE_MARKER)
}

/// C4：错误文本 → 审计可判的 `error_class`（`turn_flow` 失败事件消费）。
///
/// 取值冻结为 `timeout|rate_limited|quota|network|auth|bad_request|other`，判定优先级
/// 按 C3-S1 冻结：`quota` → `rate_limited` → `timeout` → `auth` → `network` →
/// `bad_request` → `other`。只消费本文件既有码表常量（+ 同文件措辞补充），
/// 经 [`strip_endpoint_fragments`]/[`hints_match`] 判定（禁止裸 `contains` 与第二套表）。
pub fn classify_error_class(err: &str) -> &'static str {
    let lower = err.to_ascii_lowercase();
    let stripped = strip_endpoint_fragments(&lower);
    let text = stripped.as_str();
    if hints_match(text, UPSTREAM_QUOTA_HINTS) {
        return "quota";
    }
    if hints_match(text, RATE_LIMIT_LONG_BACKOFF_HINTS) {
        return "rate_limited";
    }
    if hints_match(text, UPSTREAM_TIMEOUT_HINTS) {
        return "timeout";
    }
    if hints_match(text, UPSTREAM_AUTH_HINTS) || hints_match(text, UPSTREAM_AUTH_WORDING_HINTS) {
        return "auth";
    }
    if hints_match(text, UPSTREAM_NETWORK_HINTS) || hints_match(text, UPSTREAM_TRANSPORT_HINTS) {
        return "network";
    }
    if hints_match(text, UPSTREAM_BAD_REQUEST_HINTS)
        || hints_match(text, UPSTREAM_BAD_REQUEST_WORDING_HINTS)
    {
        return "bad_request";
    }
    "other"
}

// ── PA-172 §3.1/§3.2：跨 provider 换手判据与结构化标记（单一码表，禁止第二套关键词表） ──

/// PA-172 §3.2：换手触发判据——**复用既有码表**（G1）。
///
/// `true` 当且仅当 [`classify_error_class`] ∈ `{rate_limited, quota, timeout, network, auth}`
/// **或** [`upstream_code_for_hints`] == [`ErrorCode::UpstreamServerError`]（502/503/504/408 与
/// 边缘 5xx 520-525/527/530）。`bad_request` / `other` 不换手；HTTP **500 不换手**
/// （既有码表冻结 500 为 NonRetryable，且它不在 server_error 码内 ⇒ 换 provider 无收益）。
pub fn failover_eligible(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    if upstream_code_for_hints(&lower) == Some(ErrorCode::UpstreamServerError) {
        return true;
    }
    matches!(
        classify_error_class(err),
        "rate_limited" | "quota" | "timeout" | "network" | "auth"
    )
}

/// PA-172 §3.1：换手耗尽的结构后缀（可解析，供审计/文案；与 `retry_after=` /
/// `provider_cooldown_active=` 同族：只落在**头部段**，不受上游正文劫持）。
pub const FAILOVER_EXHAUSTED_MARKER: &str = "failover_exhausted=";

/// PA-172 F-e：给错误文本追加 `failover_exhausted=<hops>`（幂等）。
pub fn append_failover_exhausted_marker(err: &str, hops: usize) -> String {
    if parse_failover_exhausted_marker(err).is_some() {
        return err.to_string();
    }
    match err.find("响应正文") {
        Some(index) => {
            let (head, body) = err.split_at(index);
            format!(
                "{}；{FAILOVER_EXHAUSTED_MARKER}{hops}；{body}",
                head.trim_end_matches('；')
            )
        }
        None => format!("{err}；{FAILOVER_EXHAUSTED_MARKER}{hops}"),
    }
}

/// PA-172 F-e：解析换手跳数（**只读头部段**：正文段不参与，禁止正文伪造标记）。
pub fn parse_failover_exhausted_marker(err: &str) -> Option<usize> {
    let head_end = err.find("响应正文").unwrap_or(err.len());
    let head = &err[..head_end];
    let index = head.find(FAILOVER_EXHAUSTED_MARKER)?;
    let digits: String = head[index + FAILOVER_EXHAUSTED_MARKER.len()..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse::<usize>().ok()
}

/// PA-172 §3.3 F-c：**逐 attempt** 的有效超时 = `min(policy.requestTimeoutMs, remaining)`。
///
/// 换手预算是逐 attempt 的 deadline（不是"换手前查一次"的闸门）：每次候选 attempt 前重算
/// `remaining`，本函数是唯一的取值出口（禁止在编排里另写 `min`）。
pub fn failover_attempt_timeout_ms(policy_timeout_ms: u64, remaining_ms: u64) -> u64 {
    policy_timeout_ms.min(remaining_ms)
}

/// 当前 UTC 时刻（时, 分）：`rate_limit_window_ms` 的注入点（纯函数便于真值表测试）。
pub fn utc_now_hm() -> (u8, u8) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let day_secs = secs % 86_400;
    ((day_secs / 3_600) as u8, ((day_secs % 3_600) / 60) as u8)
}

/// 是否属于「应冷却 provider」的类别（`rate_limited` | `quota`）。
///
/// 唯一判据是 [`classify_error_class`]（禁止第二套关键词判定）；冷却短路文案本身含
/// `rate_limited`/`quota` 类名 ⇒ 同样命中，保证重试编排在冷却期内不再重放。
pub fn is_rate_limit_cooldown_error(err: &str) -> bool {
    matches!(classify_error_class(err), "rate_limited" | "quota")
}

/// 从错误文本解析限流恢复窗口（毫秒）。来源优先级：
/// 1. `Retry-After` 结构化后缀（[`parse_retry_after_marker`]，**只读头部段**，唯一解析点）；
/// 2. UTC 重置时刻（`not asked again before 06:01 UTC` / `resets at 06:01 UTC`），
///    按 `now_utc_hm` 求到该时刻的**同日**距离；**复位时刻已过（`target <= now`）不跨日回绕**，
///    返回 60s 软窗口（语义：窗口已过去，下一次调用正常尝试，最多再被限流一次即重新校准）。
/// 返回 `None` = 文本无任何窗口指示（调用方使用默认窗口/既有退避）。
pub fn rate_limit_window_ms(err: &str, now_utc_hm: (u8, u8)) -> Option<u64> {
    if let Some(secs) = parse_retry_after_marker(err) {
        return Some(secs.saturating_mul(1_000));
    }
    let lower = err.to_ascii_lowercase();
    let (hour, minute) = utc_reset_hint(&lower)?;
    let now_minutes = u32::from(now_utc_hm.0) * 60 + u32::from(now_utc_hm.1);
    let target = u32::from(hour) * 60 + u32::from(minute);
    // L3-A BLOCKING：**禁止跨日回绕**。复位时刻已过（target <= now）说明窗口已经过去，
    // 若回绕成 ~24h 会被 `enter_cooldown` 的 6h 上限截断 ⇒ 造成连续 6 小时 0 次 HTTP
    // 的过度熔断（比原缺陷更严重）。此处返回 60s 软窗口（含义：窗口已过，下一次调用
    // 正常尝试；最多再被限流一次即可重新校准）。
    if target <= now_minutes {
        return Some(60_000);
    }
    Some(u64::from(target - now_minutes) * 60_000)
}

/// 从错误文本提取人读的恢复时间（TUI 文案与冷却文案共用，单一格式化出口）。
/// 形态冻结：`预计 3600s 后恢复` / `预计 06:01 UTC 恢复`；`None` = 文本无可解析恢复时间。
pub fn rate_limit_recovery_hint(err: &str) -> Option<String> {
    if let Some(secs) = parse_retry_after_marker(err) {
        return Some(format!("预计 {secs}s 后恢复"));
    }
    let lower = err.to_ascii_lowercase();
    utc_reset_hint(&lower).map(|(hour, minute)| format!("预计 {hour:02}:{minute:02} UTC 恢复"))
}

/// PA-172 §5（遗留 #6）：时间敏感的恢复提示——以**真实 now** 计算剩余窗口。
///
/// 与 [`rate_limit_recovery_hint`] 的分工：
/// - `Retry-After` 结构化后缀是**相对秒数**，与 now 无关 ⇒ 两者输出逐字相同；
/// - UTC 重置时刻是**绝对时刻**，必须由 now 折算成剩余秒数（遗留 #6 的缺陷正是
///   `now_unix_ms` 被丢弃，文案永远只报绝对时刻、不随当前时刻收敛）。
///   复位时刻已过（同日 `target <= now`）与 [`rate_limit_window_ms`] 同口径：不跨日回绕，
///   只报 60s 软窗口。
pub fn rate_limit_recovery_hint_at(err: &str, now_unix_ms: u64) -> Option<String> {
    if let Some(secs) = parse_retry_after_marker(err) {
        return Some(format!("预计 {secs}s 后恢复"));
    }
    let lower = err.to_ascii_lowercase();
    let (hour, minute) = utc_reset_hint(&lower)?;
    let now_minutes = ((now_unix_ms / 1_000) % 86_400 / 60) as u32;
    let target = u32::from(hour) * 60 + u32::from(minute);
    let remaining_s = if target > now_minutes {
        u64::from(target - now_minutes) * 60
    } else {
        60
    };
    Some(format!(
        "预计 {remaining_s}s 后恢复（{hour:02}:{minute:02} UTC）"
    ))
}

/// UTC 重置时刻（`06:01 UTC` / `before 06:01 UTC` / `resets at 06:01 UTC`）→ (时, 分)。
/// 只按字符边界回看 `utc` 前 12 个字符，禁止按字节偏移切片（入参含上游可控文本）。
fn utc_reset_hint(lower: &str) -> Option<(u8, u8)> {
    for (index, _) in lower.match_indices("utc") {
        let mut window: Vec<char> = lower[..index].chars().rev().take(12).collect();
        window.reverse();
        if window.len() < 5 {
            continue;
        }
        for start in 0..=window.len() - 5 {
            let candidate = &window[start..start + 5];
            let is_clock = candidate[0].is_ascii_digit()
                && candidate[1].is_ascii_digit()
                && candidate[2] == ':'
                && candidate[3].is_ascii_digit()
                && candidate[4].is_ascii_digit();
            if !is_clock {
                continue;
            }
            let hour = (candidate[0] as u8 - b'0') * 10 + (candidate[1] as u8 - b'0');
            let minute = (candidate[3] as u8 - b'0') * 10 + (candidate[4] as u8 - b'0');
            if hour < 24 && minute < 60 {
                return Some((hour, minute));
            }
        }
    }
    None
}

/// Convenience function: run a full retry loop with the given policy.
/// Returns `Ok(result)` on success, or the last `RetryDecision` on exhaustion.
pub fn retry_with_policy<T, F>(
    label: &str,
    policy: &ProviderRetryPolicy,
    mut operation: F,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<T, RetryDecision>
where
    F: FnMut() -> Result<T, (String, StreamState)>,
{
    let mut budget = RetryBudget::new(policy.config.max_retries, policy.config.total_budget_ms);
    let mut last_decision = RetryDecision::Abort {
        reason: "no attempts made".to_string(),
    };
    let start = std::time::Instant::now();

    for attempt in 0..=policy.config.max_retries {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(RetryDecision::Abort {
                reason: "cancelled".to_string(),
            });
        }

        let delay = compute_delay(attempt, &policy.config, 0.5);
        if attempt > 0 {
            if let Err(d) = policy.sleeper.sleep(delay, cancelled) {
                return Err(RetryDecision::Abort {
                    reason: format!("sleep cancelled: {:?}", d),
                });
            }
            budget.record_attempt(delay.as_millis() as u64);
        }

        match operation() {
            Ok(value) => {
                if attempt > 0 {
                    eprintln!(
                        "{}: succeeded on attempt {}/{}",
                        label, attempt, policy.config.max_retries
                    );
                }
                budget.record_execution(start.elapsed().as_millis() as u64);
                return Ok(value);
            }
            Err((err, stream_state)) => {
                let failure = policy.classify(&err);
                if attempt > 0 {
                    eprintln!(
                        "{}: attempt {}/{} failed: {} (class={:?})",
                        label, attempt, policy.config.max_retries, err, failure
                    );
                }
                let decision = policy.decide(&failure, &budget, attempt, None, stream_state);
                match &decision {
                    RetryDecision::Retry { .. } => {
                        last_decision = decision;
                        // continue loop
                    }
                    RetryDecision::Fallback { .. }
                    | RetryDecision::Escalate { .. }
                    | RetryDecision::Abort { .. } => {
                        return Err(decision);
                    }
                }
            }
        }
    }
    Err(last_decision)
}

// ─────────────────────── F2：per-call 纯净窗口（重试账本） ───────────────────────
//
// 取证与裁决见 `.dev-team/diag-tps/fix/f2f3-green.md`（I1）。一句话：per-call 的
// `turn_duration_ms` / `first_token_latency_ms` 原本在重试循环**之外**取样
// （`runtime/turn_stream.rs::initial_decision_started_at` / `provider_call_started_at`、
// `runtime/tool_recovery.rs::started_at`），而重试在
// `provider::retry_provider_scoped_with_report_config` 的循环里（429/525 单次
// 1.9–8.0s，外加 20s/40s 限流退避）⇒ 失败尝试耗时被算进 `dur`。
// `gen = dur − ttft` 因两端同步被抬高而近似抵消；但 `end_to_end` 口径
// （`Σout / Σdur`）会被系统性压低（实测 gemini e2e 10.4 t/s vs decode 98.6 t/s）。
//
// 本账本由**重试循环**逐 attempt 记账，调用方只在 provider 调用前后各加一行：
// 调用前 `let outer_start = retry::begin_provider_attempt_window();`，
// 调用后 `let pure = retry::take_pure_call_window(outer_start, outer_ttft);`。
// I3 不变量：本模块**不算速度**——速度唯一出口仍是
// `turn_flow::{decode_eligible, call_speed_from_raw}`，本账本只负责把窗口修纯净。

/// 一次 provider 调用（可能含多次 attempt / 换手）的重试账本。
///
/// 「失败耗时」与「attempts 次数」在这里是**独立字段**：它们绝不参与 per-call 的
/// `dur` / `gen`，只做取证与结构化留痕（I2）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderAttemptWindow {
    /// attempt 次数（含最终成功的那次；整体失败的调用计全部尝试次数）。
    pub attempts: u32,
    /// 失败 attempt **自身**耗时之和（毫秒，不含退避 sleep）。
    pub failed_attempt_ms: u64,
    /// 重试循环内的退避 sleep 累计（毫秒）。
    pub slept_ms: u64,
    /// 最后一次**成功** attempt 的耗时（毫秒）；无成功 attempt ⇒ `None`。
    pub succeeded_attempt_ms: Option<u64>,
    /// 最后一次成功 attempt 的**起点**相对账本起点的偏移（毫秒）。
    pub succeeded_attempt_offset_ms: Option<u64>,
}

impl ProviderAttemptWindow {
    /// 成功 attempt 之前累计的耗时（失败尝试 + 退避）。
    pub fn prelude_ms(&self) -> u64 {
        self.failed_attempt_ms.saturating_add(self.slept_ms)
    }

    /// 外层 `dur`（跨 attempt 墙钟）→ 纯净 per-call `dur`（只覆盖最后一次成功 attempt）。
    ///
    /// 无成功 attempt（调用整体失败）⇒ 原样返回外层值，**不伪造**纯净性。
    pub fn rebase_duration_ms(&self, outer_duration_ms: u64) -> u64 {
        self.succeeded_attempt_ms.unwrap_or(outer_duration_ms)
    }

    /// 外层 `ttft`（跨 attempt）→ 纯净 `ttft`（相对成功 attempt 起点）。
    ///
    /// 无成功 attempt ⇒ 原样返回（同上，不伪造）。毫秒级取整可能让结果落到 0
    /// （首 token 在成功 attempt 起点当毫秒内到达）——0 是**诚实值**，不是缺失。
    pub fn rebase_first_token_ms(&self, outer_first_token_ms: Option<u64>) -> Option<u64> {
        outer_first_token_ms.map(|ttft| {
            ttft.saturating_sub(self.succeeded_attempt_offset_ms.unwrap_or(0))
        })
    }
}

/// 账本内部可变状态（读取即消费，一次性）。
#[derive(Debug, Clone, Copy)]
struct AttemptWindowState {
    started_at: Instant,
    attempts: u32,
    failed_attempt_ms: u64,
    slept_ms: u64,
    succeeded_attempt_ms: Option<u64>,
    succeeded_attempt_offset_ms: Option<u64>,
}

impl AttemptWindowState {
    fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            attempts: 0,
            failed_attempt_ms: 0,
            slept_ms: 0,
            succeeded_attempt_ms: None,
            succeeded_attempt_offset_ms: None,
        }
    }

    fn snapshot(&self) -> ProviderAttemptWindow {
        ProviderAttemptWindow {
            attempts: self.attempts,
            failed_attempt_ms: self.failed_attempt_ms,
            slept_ms: self.slept_ms,
            succeeded_attempt_ms: self.succeeded_attempt_ms,
            succeeded_attempt_offset_ms: self.succeeded_attempt_offset_ms,
        }
    }
}

// 为什么用 thread_local：一次 provider 调用（含其内部重试/换手）在**同一线程上同步执行**
// ——`on_delta` 也在阻塞 SSE 读循环内联调用（`provider::openai_sse::push_payload`）。
// 与 `runtime::limits` 的测试覆写同一取舍：不引入跨调用共享状态，也不污染并行用例。
thread_local! {
    static ATTEMPT_WINDOW: RefCell<Option<AttemptWindowState>> = const { RefCell::new(None) };
}

/// 开启一次 provider 调用的重试账本，返回**与外层计时同源**的起点。
///
/// 调用方必须以返回的 `Instant` 作为 `dur` / `ttft` 的计时起点（两者必须同源，
/// 否则 [`ProviderAttemptWindow::rebase_first_token_ms`] 会漂移）。
pub fn begin_provider_attempt_window() -> Instant {
    let started_at = Instant::now();
    ATTEMPT_WINDOW.with(|slot| *slot.borrow_mut() = Some(AttemptWindowState::new(started_at)));
    started_at
}

/// 取走本线程当前调用的账本（一次性；取走后回到「未开启」状态）。
pub fn take_provider_attempt_window() -> Option<ProviderAttemptWindow> {
    ATTEMPT_WINDOW.with(|slot| slot.borrow_mut().take().map(|state| state.snapshot()))
}

/// 记一次**失败** attempt（供重试循环调用；未开启账本 ⇒ 静默 no-op）。
pub fn note_provider_attempt_failed(elapsed_ms: u64) {
    ATTEMPT_WINDOW.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.attempts = state.attempts.saturating_add(1);
            state.failed_attempt_ms = state.failed_attempt_ms.saturating_add(elapsed_ms);
        }
    });
}

/// 记一次**成功** attempt（供重试循环调用；未开启账本 ⇒ 静默 no-op）。
///
/// `started_at` 是本次 attempt 的起点：由它算出成功 attempt 相对账本起点的偏移，
/// 从而把外层 `ttft` 平移到「成功 attempt 内部」。
pub fn note_provider_attempt_succeeded(started_at: Instant, elapsed_ms: u64) {
    ATTEMPT_WINDOW.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.attempts = state.attempts.saturating_add(1);
            state.succeeded_attempt_ms = Some(elapsed_ms);
            state.succeeded_attempt_offset_ms = Some(
                started_at
                    .saturating_duration_since(state.started_at)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64,
            );
        }
    });
}

/// 记重试循环内一次退避 sleep 的时长（未开启账本 ⇒ 静默 no-op）。
pub fn note_provider_attempt_backoff_slept(slept_ms: u64) {
    ATTEMPT_WINDOW.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            state.slept_ms = state.slept_ms.saturating_add(slept_ms);
        }
    });
}

// ── 上游重试的**实时**观测出口（2026-11-14） ─────────────────────────────────
//
// 需求：重试进度必须出现在界面的状态消息里（此前只有 `provider_log` 落盘，界面 0 信号；
// 历史上界面曾伪造「重试中(1/5)」，因为拿不到真实分母）。
//
// 为什么是 thread_local 观察者而不是事件 sink：重试循环在库层深处，拿不到 turn 层的
// `TurnEventSink`（sink 以 `&impl TurnEventSink` 逐层传参，无法存进 'static 的
// thread_local），而一次 provider 调用（含其内部重试与换手）与 `on_delta` 回调都在
// **同一线程同步执行** —— 与本文件 `ATTEMPT_WINDOW` 完全同一取舍。故由宿主（TUI 轮次
// 线程 / 桌面端 spawn_blocking 任务）安装一个通知闭包即可拿到实时进度；
// 未安装 ⇒ 静默 no-op（CLI、测试、子代理线程零副作用）。
thread_local! {
    static RETRY_OBSERVER: RefCell<Option<RetryObserver>> = const { RefCell::new(None) };
}

/// 一次「即将重试」的事实。`attempt` = **已失败**的尝试次数（1 起），
/// `max_attempts` = 本跳允许的尝试总数（含首次）⇒ 界面口径 `attempt/max_attempts`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryNotice {
    /// provider 调用名（`decision` / `decision_stream` / `followup` …），与 `provider_log` 同源。
    pub label: String,
    /// 重试作用域（`decision` / `followup`）。
    pub scope: &'static str,
    /// 已失败的尝试次数（含首次失败），1 起。
    pub attempt: u32,
    /// 本跳允许的尝试总数（含首次）。
    pub max_attempts: u32,
    /// 本次退避将 sleep 的毫秒数（尚未执行）。
    pub delay_ms: u64,
    /// 失败分类（`retry::classify_error_class` 口径，用于区分限流/超时/服务端）。
    pub error_class: String,
}

/// 重试通知观察者（宿主安装；签名刻意用 `&RetryNotice` 以便零拷贝）。
pub type RetryObserver = Box<dyn Fn(&RetryNotice)>;

/// 观察者守卫：生命周期 = 宿主轮次线程生命周期，`drop` 即摘除（避免后续调用误投递）。
pub struct RetryObserverGuard {
    installed: bool,
}

impl RetryObserverGuard {
    /// 未安装任何观察者时的空守卫（便于调用方统一持有）。
    pub fn idle() -> Self {
        Self { installed: false }
    }
}

impl Drop for RetryObserverGuard {
    fn drop(&mut self) {
        if self.installed {
            RETRY_OBSERVER.with(|slot| {
                *slot.borrow_mut() = None;
            });
        }
    }
}

/// 在当前线程安装重试通知观察者（覆盖既有安装；同线程仅需一个）。
pub fn install_retry_observer(observer: RetryObserver) -> RetryObserverGuard {
    RETRY_OBSERVER.with(|slot| {
        *slot.borrow_mut() = Some(observer);
    });
    RetryObserverGuard { installed: true }
}

/// 派发一次「即将重试」通知（未安装观察者 ⇒ 静默 no-op）。
///
/// 观察者必须**非阻塞**（生产实现只往 channel 里 `send`），否则会拖慢退避调度。
pub fn notify_provider_retry(notice: &RetryNotice) {
    RETRY_OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow().as_ref() {
            observer(notice);
        }
    });
}

/// 「即将重试」通知的装配器：绑定 provider 调用名与作用域，供重试循环逐次派发。
///
/// 只报告**真实发生**的重试（调用点都在 `RetryDecision::Retry` 分支内、sleep 之前）；
/// 分母 `max_attempts` = `max_retries + 1`（含首次），与分子同源，不虚构进度。
pub struct RetryNotifier {
    label: String,
    scope: &'static str,
}

impl RetryNotifier {
    /// 绑定 provider 调用名（`decision` / `followup` …）与重试作用域。
    pub fn new(label: &str, scope: &'static str) -> Self {
        Self {
            label: label.to_string(),
            scope,
        }
    }

    /// 派发一次通知（未安装观察者 ⇒ no-op）。`attempt` = 已失败次数（1 起）。
    pub fn notify(
        &self,
        attempt: u32,
        config: &BackoffConfig,
        last_error: &str,
        next_attempt: u32,
    ) {
        notify_provider_retry(&RetryNotice {
            label: self.label.clone(),
            scope: self.scope,
            attempt,
            max_attempts: config.max_retries.saturating_add(1),
            delay_ms: compute_delay(next_attempt, config, 0.5).as_millis() as u64,
            error_class: classify_error_class(last_error).to_string(),
        });
    }
}

/// 纯净 per-call 窗口的结算结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PureCallWindow {
    /// 只覆盖最后一次成功 attempt 的耗时（毫秒）。喂 `build_provider_call_cache_record`
    /// 的 `turn_duration_ms`。
    pub duration_ms: u64,
    /// 只覆盖最后一次成功 attempt 的首 token 延迟（毫秒）。喂同记录的
    /// `first_token_latency_ms`。
    pub first_token_latency_ms: Option<u64>,
    /// 原始账本（重试次数 / 失败耗时 / 退避，**独立字段**，绝不混入 `gen`）。
    /// `None` ⇔ 未开启账本（第三方直接调用 provider / 测试桩）。
    pub window: Option<ProviderAttemptWindow>,
}

impl PureCallWindow {
    /// 纯净窗口是否真的排除了失败尝试（有重试且已定位成功 attempt）。
    ///
    /// 供结构化留痕与用例断言用；不参与速度公式。
    pub fn retried(&self) -> bool {
        self.window
            .is_some_and(|window| window.succeeded_attempt_ms.is_some() && window.attempts > 1)
    }
}

/// 由外层观测结算纯净 per-call 窗口（**消费**账本，一次性）。
///
/// `outer_started_at` 必须是 [`begin_provider_attempt_window`] 的返回值。
/// 未开启账本 ⇒ 原样返回外层观测（不伪造纯净性，也不回填 0）。
pub fn take_pure_call_window(
    outer_started_at: Instant,
    outer_first_token_ms: Option<u64>,
) -> PureCallWindow {
    let outer_duration_ms = outer_started_at.elapsed().as_millis() as u64;
    let Some(window) = take_provider_attempt_window() else {
        return PureCallWindow {
            duration_ms: outer_duration_ms,
            first_token_latency_ms: outer_first_token_ms,
            window: None,
        };
    };
    let pure = PureCallWindow {
        duration_ms: window.rebase_duration_ms(outer_duration_ms),
        first_token_latency_ms: window.rebase_first_token_ms(outer_first_token_ms),
        window: Some(window),
    };
    // I2：attempts 与失败耗时**独立留痕**（结构化日志），不进 per-call dur/gen。
    if pure.retried() {
        log_pure_call_window(outer_duration_ms, outer_first_token_ms, &pure);
    }
    pure
}

/// 重试窗口的结构化留痕（一行一条调用；无重试不写，避免常态噪声）。
fn log_pure_call_window(
    outer_duration_ms: u64,
    outer_first_token_ms: Option<u64>,
    pure: &PureCallWindow,
) {
    let Some(window) = pure.window else {
        return;
    };
    crate::agent::otel::log::log_json_with_fields(
        "info",
        "provider_call_retry_window",
        None,
        &[
            ("attempts", serde_json::json!(window.attempts)),
            ("failed_attempt_ms", serde_json::json!(window.failed_attempt_ms)),
            ("slept_ms", serde_json::json!(window.slept_ms)),
            ("prelude_ms", serde_json::json!(window.prelude_ms())),
            (
                "succeeded_attempt_offset_ms",
                serde_json::json!(window.succeeded_attempt_offset_ms),
            ),
            (
                "pure_duration_ms",
                serde_json::json!(pure.duration_ms),
            ),
            (
                "pure_first_token_latency_ms",
                serde_json::json!(pure.first_token_latency_ms),
            ),
            ("outer_duration_ms", serde_json::json!(outer_duration_ms)),
            (
                "outer_first_token_latency_ms",
                serde_json::json!(outer_first_token_ms),
            ),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_delay_first_attempt_is_zero() {
        let cfg = BackoffConfig::default();
        assert_eq!(compute_delay(0, &cfg, 0.0), Duration::ZERO);
    }

    #[test]
    fn compute_delay_exponential() {
        let cfg = BackoffConfig {
            initial_delay_ms: 500,
            multiplier: 2.0,
            max_delay_ms: 8000,
            jitter_kind: JitterKind::None,
            ..Default::default()
        };
        assert_eq!(compute_delay(1, &cfg, 0.0).as_millis(), 500);
        assert_eq!(compute_delay(2, &cfg, 0.0).as_millis(), 1000);
        assert_eq!(compute_delay(3, &cfg, 0.0).as_millis(), 2000);
        assert_eq!(compute_delay(4, &cfg, 0.0).as_millis(), 4000);
        assert_eq!(compute_delay(5, &cfg, 0.0).as_millis(), 8000);
    }

    #[test]
    fn compute_delay_respects_max() {
        let cfg = BackoffConfig {
            initial_delay_ms: 1000,
            multiplier: 4.0,
            max_delay_ms: 3000,
            jitter_kind: JitterKind::None,
            ..Default::default()
        };
        assert_eq!(compute_delay(2, &cfg, 0.0).as_millis(), 3000);
    }

    #[test]
    fn stream_state_transitions() {
        let s = StreamState::PreConnection;
        assert_eq!(
            s.transition(StreamEventKind::ConnectionEstablished),
            StreamState::NoDelta
        );
        let s = StreamState::NoDelta;
        assert_eq!(
            s.transition(StreamEventKind::Reasoning),
            StreamState::ReasoningOnly
        );
        let s = StreamState::ReasoningOnly;
        assert_eq!(
            s.transition(StreamEventKind::Text),
            StreamState::VisibleTextStarted
        );
        let s = StreamState::ReasoningOnly;
        assert_eq!(
            s.transition(StreamEventKind::MixedReasoningText),
            StreamState::VisibleTextStarted
        );
        let s = StreamState::NoDelta;
        assert_eq!(
            s.transition(StreamEventKind::ToolCall),
            StreamState::ToolCallStarted
        );
    }

    #[test]
    fn stream_state_may_auto_retry() {
        assert!(StreamState::PreConnection.may_auto_retry());
        assert!(StreamState::NoDelta.may_auto_retry());
        assert!(StreamState::ReasoningOnly.may_auto_retry());
        assert!(!StreamState::VisibleTextStarted.may_auto_retry());
        assert!(!StreamState::ToolCallStarted.may_auto_retry());
    }

    #[test]
    fn retry_budget_exhausted_attempt_limit() {
        let mut budget = RetryBudget::new(2, 10000);
        assert!(!budget.is_exhausted());
        budget.record_attempt(100);
        assert!(!budget.is_exhausted());
        budget.record_attempt(100);
        assert!(budget.is_exhausted());
        assert_eq!(budget.exhausted_reason(), Some("attempt_limit_exhausted"));
    }

    #[test]
    fn retry_budget_exhausted_time() {
        let mut budget = RetryBudget::new(10, 1000);
        assert!(!budget.is_exhausted());
        budget.record_attempt(1100);
        assert!(budget.is_exhausted());
        assert_eq!(budget.exhausted_reason(), Some("time_budget_exhausted"));
    }

    #[test]
    fn failure_classification_timeout_is_retryable() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let kind = policy.classify("operation timed out; type=timeout");
        assert!(kind.is_retryable());
    }

    #[test]
    fn failure_classification_400_is_non_retryable() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let kind = policy.classify("HTTP 400 Bad Request");
        assert!(!kind.is_retryable());
        assert!(matches!(kind, FailureKind::NonRetryable { .. }));
    }

    #[test]
    fn failure_classification_context_too_large_is_requires_mutation() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let kind = policy.classify("context too large: 128k tokens");
        assert!(!kind.is_retryable());
        assert!(matches!(kind, FailureKind::RequiresRequestMutation { .. }));
    }

    #[test]
    fn failure_classification_timeout_at_visible_text_is_unsafe_via_decide() {
        // classify no longer checks stream state; decide does.
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let kind = policy.classify("timeout");
        assert!(kind.is_retryable());
        let budget = RetryBudget::new(3, 10000);
        // decide sees VisibleTextStarted and aborts even for retryable errors
        let decision = policy.decide(&kind, &budget, 1, None, StreamState::VisibleTextStarted);
        assert!(matches!(decision, RetryDecision::Abort { .. }));
    }

    #[test]
    fn retry_after_parse_seconds() {
        let ra = RetryAfter::parse("30");
        assert_eq!(ra.seconds, Some(30));
    }

    #[test]
    fn retry_after_parse_zero() {
        let ra = RetryAfter::parse("0");
        assert!(ra.seconds.is_some());
        let delay = ra.effective_delay(500, 10000);
        assert_eq!(delay, None);
    }

    #[test]
    fn retry_after_exceeds_budget() {
        let ra = RetryAfter { seconds: Some(60) };
        let delay = ra.effective_delay(500, 10000);
        assert_eq!(delay, None);
    }

    #[test]
    fn retry_after_effective_delay() {
        let ra = RetryAfter { seconds: Some(3) };
        let delay = ra.effective_delay(500, 10000);
        assert_eq!(delay, Some(3000));
    }

    #[test]
    fn decide_aborts_on_non_retryable() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::NonRetryable {
            details: "400".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        let decision = policy.decide(&failure, &budget, 1, None, StreamState::NoDelta);
        assert!(matches!(decision, RetryDecision::Abort { .. }));
    }

    #[test]
    fn decide_escalates_on_exhausted_budget() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::TransientRetryable {
            details: "timeout".to_string(),
        };
        let mut budget = RetryBudget::new(1, 10000);
        budget.record_attempt(500);
        let decision = policy.decide(&failure, &budget, 2, None, StreamState::NoDelta);
        assert!(matches!(decision, RetryDecision::Escalate { .. }));
    }

    #[test]
    fn decide_aborts_on_unsafe_with_any_state() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::UnsafeToRetry {
            details: "stream error".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        // UnsafeToRetry + any state → Abort (may_auto_retry doesn't matter for UnsafeToRetry)
        let decision = policy.decide(&failure, &budget, 1, None, StreamState::NoDelta);
        assert!(matches!(decision, RetryDecision::Abort { .. }));
    }

    #[test]
    fn decide_aborts_on_visible_text_state() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::TransientRetryable {
            details: "timeout".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        // VisibleTextStarted prohibits auto-retry even for transient errors
        let decision = policy.decide(&failure, &budget, 1, None, StreamState::VisibleTextStarted);
        assert!(matches!(decision, RetryDecision::Abort { .. }));
    }

    #[test]
    fn should_fallback_to_sync_returns_true_for_non_retryable_in_reasoning() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::NonRetryable {
            details: "server error".to_string(),
        };
        assert!(policy.should_fallback_to_sync(&failure, StreamState::ReasoningOnly));
        assert!(policy.should_fallback_to_sync(&failure, StreamState::NoDelta));
    }

    #[test]
    fn should_fallback_to_sync_returns_false_for_visible_text() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::NonRetryable {
            details: "server error".to_string(),
        };
        assert!(!policy.should_fallback_to_sync(&failure, StreamState::VisibleTextStarted));
        assert!(!policy.should_fallback_to_sync(&failure, StreamState::ToolCallStarted));
    }

    #[test]
    fn decide_retries_on_retryable() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::TransientRetryable {
            details: "timeout".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        let decision = policy.decide(&failure, &budget, 1, None, StreamState::NoDelta);
        assert!(matches!(decision, RetryDecision::Retry { .. }));
    }

    #[test]
    fn decide_aborts_on_requires_mutation() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::RequiresRequestMutation {
            details: "context too large".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        let decision = policy.decide(&failure, &budget, 1, None, StreamState::NoDelta);
        assert!(matches!(decision, RetryDecision::Abort { .. }));
    }

    #[test]
    fn failure_kind_details_is_pub() {
        let f = FailureKind::TransientRetryable {
            details: "test-detail".to_string(),
        };
        assert_eq!(f.details(), "test-detail");
    }

    #[test]
    fn fake_sleeper_records_total() {
        use std::sync::atomic::AtomicBool;
        let sleeper = FakeSleeper::new();
        let cancelled = AtomicBool::new(false);
        assert!(sleeper
            .sleep(Duration::from_millis(150), &cancelled)
            .is_ok());
        assert_eq!(
            sleeper
                .total_slept
                .load(std::sync::atomic::Ordering::Relaxed),
            150
        );
    }

    #[test]
    fn retry_with_policy_succeeds_on_first_try() {
        use std::sync::atomic::AtomicBool;
        let policy = ProviderRetryPolicy::with_sleeper(
            BackoffConfig::default(),
            Box::new(FakeSleeper::new()),
        );
        let cancelled = AtomicBool::new(false);
        let result = retry_with_policy(
            "test",
            &policy,
            || Ok::<_, (String, StreamState)>(42),
            &cancelled,
        );
        assert_eq!(result, Ok(42));
    }

    #[test]
    fn retry_with_policy_aborts_on_non_retryable() {
        use std::sync::atomic::AtomicBool;
        let config = BackoffConfig {
            max_retries: 1,
            ..Default::default()
        };
        let policy = ProviderRetryPolicy::with_sleeper(config, Box::new(FakeSleeper::new()));
        let cancelled = AtomicBool::new(false);
        let result = retry_with_policy(
            "test",
            &policy,
            || Err::<i32, _>(("HTTP 400".to_string(), StreamState::NoDelta)),
            &cancelled,
        );
        assert!(matches!(result, Err(RetryDecision::Abort { .. })));
    }

    #[test]
    fn retry_with_policy_exhausts_and_escalates() {
        use std::sync::atomic::AtomicBool;
        let config = BackoffConfig {
            max_retries: 3,
            initial_delay_ms: 1,
            ..Default::default()
        };
        let policy = ProviderRetryPolicy::with_sleeper(config, Box::new(FakeSleeper::new()));
        let cancelled = AtomicBool::new(false);
        let tries = std::cell::Cell::new(0);
        let result = retry_with_policy(
            "test",
            &policy,
            || {
                tries.set(tries.get() + 1);
                Err::<i32, _>(("timeout".to_string(), StreamState::NoDelta))
            },
            &cancelled,
        );
        assert!(matches!(result, Err(RetryDecision::Escalate { .. })));
        // max_retries=3 means: 1 initial + up to 3 retries = 4 total calls
        assert_eq!(tries.get(), 4);
    }

    #[test]
    fn retry_after_parse_invalid_returns_none() {
        let ra = RetryAfter::parse("not-a-number");
        assert!(ra.seconds.is_none());
    }

    #[test]
    fn retry_with_policy_cancelled_returns_abort_immediately() {
        use std::sync::atomic::AtomicBool;
        let config = BackoffConfig {
            max_retries: 3,
            ..Default::default()
        };
        let policy = ProviderRetryPolicy::with_sleeper(config, Box::new(FakeSleeper::new()));
        let cancelled = AtomicBool::new(true);
        let tries = std::cell::Cell::new(0);
        let result = retry_with_policy(
            "test",
            &policy,
            || {
                tries.set(tries.get() + 1);
                Err::<i32, _>(("timeout".to_string(), StreamState::NoDelta))
            },
            &cancelled,
        );
        assert!(matches!(result, Err(RetryDecision::Abort { .. })));
        // Should not have called the operation at all
        assert_eq!(tries.get(), 0);
    }

    #[test]
    fn stream_state_transition_policing_prevents_rollback() {
        // Once visible text started, no event should roll back
        let s = StreamState::VisibleTextStarted;
        assert_eq!(
            s.transition(StreamEventKind::Reasoning),
            StreamState::VisibleTextStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::Text),
            StreamState::VisibleTextStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::MixedReasoningText),
            StreamState::VisibleTextStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::ToolCall),
            StreamState::ToolCallStarted
        );
        // Once tool call started, no event should roll back
        let s = StreamState::ToolCallStarted;
        assert_eq!(
            s.transition(StreamEventKind::Reasoning),
            StreamState::ToolCallStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::Text),
            StreamState::ToolCallStarted
        );
        // PreConnection direct to tool call is allowed
        let s = StreamState::PreConnection;
        assert_eq!(
            s.transition(StreamEventKind::ToolCall),
            StreamState::ToolCallStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::Text),
            StreamState::VisibleTextStarted
        );
        // NoDelta text directly jumps to visible text
        let s = StreamState::NoDelta;
        assert_eq!(
            s.transition(StreamEventKind::Text),
            StreamState::VisibleTextStarted
        );
        assert_eq!(
            s.transition(StreamEventKind::MixedReasoningText),
            StreamState::VisibleTextStarted
        );
    }

    #[test]
    fn decide_uses_retry_after_parameter() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let failure = FailureKind::TransientRetryable {
            details: "timeout".to_string(),
        };
        let budget = RetryBudget::new(3, 10000);
        let ra = RetryAfter { seconds: Some(2) };
        let decision = policy.decide(&failure, &budget, 1, Some(&ra), StreamState::NoDelta);
        // Should still be Retry (retryable + budget available)
        assert!(matches!(decision, RetryDecision::Retry { .. }));
        // Delay should be influenced by RetryAfter
        if let RetryDecision::Retry { delay_ms } = &decision {
            assert!(*delay_ms >= 2000, "retry-after should influence delay");
        }
    }

    #[test]
    fn retry_budget_record_execution() {
        let mut budget = RetryBudget::new(3, 10000);
        assert_eq!(budget.elapsed_ms, 0);
        budget.record_execution(2500);
        assert_eq!(budget.elapsed_ms, 2500);
        budget.record_execution(500);
        assert_eq!(budget.elapsed_ms, 3000);
    }

    #[test]
    fn failure_kind_methods() {
        let u = FailureKind::UnsafeToRetry {
            details: "mid-stream".to_string(),
        };
        assert!(u.is_unsafe_to_retry());
        assert!(!u.requires_request_mutation());
        let m = FailureKind::RequiresRequestMutation {
            details: "context too large".to_string(),
        };
        assert!(m.requires_request_mutation());
        assert!(!m.is_unsafe_to_retry());
    }

    #[test]
    fn jitter_kind_full_edge_values() {
        let cfg = BackoffConfig {
            initial_delay_ms: 1000,
            multiplier: 2.0,
            max_delay_ms: 8000,
            jitter_kind: JitterKind::Full,
            ..Default::default()
        };
        // jitter_sample=0.0 → 0ms delay
        assert_eq!(compute_delay(1, &cfg, 0.0).as_millis(), 0);
        // jitter_sample=1.0 → full delay
        assert_eq!(compute_delay(1, &cfg, 1.0).as_millis(), 1000);
    }

    #[test]
    fn jitter_kind_equal_edge_values() {
        let cfg = BackoffConfig {
            initial_delay_ms: 1000,
            multiplier: 2.0,
            max_delay_ms: 8000,
            jitter_kind: JitterKind::Equal,
            ..Default::default()
        };
        // jitter_sample=0.0 → base/2 = 500ms
        assert_eq!(compute_delay(1, &cfg, 0.0).as_millis(), 500);
        // jitter_sample=1.0 → base = 1000ms
        assert_eq!(compute_delay(1, &cfg, 1.0).as_millis(), 1000);
    }

    // ── Stage2 R1 锁定：八分组逐项一致 + 码表唯一来源 ──

    #[test]
    fn stage2_upstream_code_for_hints_covers_r1_groups() {
        use crate::agent::error_code::ErrorCode::*;
        // ① timeout 四词
        for probe in [
            "type=timeout",
            "operation timeout",
            "timed out waiting",
            "deadline has elapsed",
        ] {
            assert_eq!(
                upstream_code_for_hints(probe),
                Some(UpstreamTimeout),
                "{probe}"
            );
        }
        // ② 429 三词
        for probe in ["429", "rate limit hit", "rate_limit hit"] {
            assert_eq!(
                upstream_code_for_hints(probe),
                Some(UpstreamRateLimited),
                "{probe}"
            );
        }
        // ③ 5xx/408
        for probe in ["502", "503", "504", "408"] {
            assert_eq!(
                upstream_code_for_hints(probe),
                Some(UpstreamServerError),
                "{probe}"
            );
        }
        // ④ 连接三词（禁扩 connect 泛词："connect" 不命中）
        for probe in ["connection reset", "connection refused", "dns failure"] {
            assert_eq!(
                upstream_code_for_hints(probe),
                Some(UpstreamNetwork),
                "{probe}"
            );
        }
        assert_eq!(upstream_code_for_hints("connect"), None);
        // ⑤ 400 系七码（401/403 走 UpstreamAuth，同落 NonRetryable）
        for probe in ["400", "404", "422", "407", "413"] {
            assert_eq!(
                upstream_code_for_hints(probe),
                Some(UpstreamBadRequest),
                "{probe}"
            );
        }
        for probe in ["401", "403"] {
            assert_eq!(
                upstream_code_for_hints(probe),
                Some(UpstreamAuth),
                "{probe}"
            );
        }
        // ⑥ context 四词
        for probe in [
            "context too large",
            "context_length exceeded",
            "max_tokens hit",
            "payload too large",
        ] {
            assert_eq!(
                upstream_code_for_hints(probe),
                Some(UpstreamContextOverflow),
                "{probe}"
            );
        }
        // ⑦ 其余默认 None
        assert_eq!(upstream_code_for_hints("provider returned 500"), None);
    }

    // ── ADR-0086：边缘 5xx + 传输层标志（实证驱动的分类补齐） ──

    #[test]
    fn edge_5xx_maps_to_server_error_but_origin_tls_stays_abort() {
        use crate::agent::error_code::ErrorCode::*;
        // ③′ 边缘 5xx：全部落 UpstreamServerError（可重试）
        // 525 于 2026-11-12 由「排除」改为「纳入」——源站握手瞬时失败可自愈，
        // 与 522 同组；判定理由见 `UPSTREAM_EDGE_SERVER_ERROR_HINTS` 注释与对应 ADR。
        for code in ["520", "521", "522", "523", "524", "525", "527", "530"] {
            let probe = format!(
                "provider 返回错误状态：{code} <unknown status code>；耗时=6401ms；响应正文："
            );
            assert_eq!(
                upstream_code_for_hints(&probe),
                Some(UpstreamServerError),
                "{code}"
            );
        }
        // 526 Invalid SSL Certificate（源站证书无效）：保持 NonRetryable（码表不命中 → None）
        for code in ["526"] {
            let probe = format!(
                "provider 返回错误状态：{code} <unknown status code>；耗时=6097ms；响应正文："
            );
            assert_eq!(upstream_code_for_hints(&probe), None, "{code}");
        }
        // 500 冻结不变；码值粘在更长数字里仍不得命中
        assert_eq!(upstream_code_for_hints("elapsed=52201ms"), None);
        assert_eq!(upstream_code_for_hints("token id 1522"), None);
        // 525 不得靠子串误命中：`1525` / `5250ms` 都不是状态码
        assert_eq!(upstream_code_for_hints("token id 1525"), None);
        assert_eq!(upstream_code_for_hints("elapsed=5250ms"), None);
    }

    #[test]
    fn transport_flags_map_to_network_after_status_code_judgement() {
        use crate::agent::error_code::ErrorCode::*;
        // ④′ 无状态码的传输阶段失败 → UpstreamNetwork（可重试）
        let connect = "调用 provider 流式接口失败：error sending request for url (http://10.42.0.67:8080/v1/chat/completions)；type=connect+request；elapsed=3108ms；source=client error (Connect refused)";
        assert_eq!(upstream_code_for_hints(connect), Some(UpstreamNetwork));
        let send = "调用 provider 失败：error sending request for url (https://tokens.ponyjob.top/v1/chat/completions)；type=request；elapsed=30996ms；source=client error (SendRequest)";
        assert_eq!(upstream_code_for_hints(send), Some(UpstreamNetwork));
        // 既有约束不变：裸 `connect` 泛词不命中
        assert_eq!(upstream_code_for_hints("connect"), None);
        // 传输标志含真实 400 → 状态码判据优先，仍是 NonRetryable
        let bad_request =
            "provider 返回错误状态：400；type=request；elapsed=1200ms；响应正文：invalid model";
        assert_eq!(
            upstream_code_for_hints(bad_request),
            Some(UpstreamBadRequest)
        );
        // type=decode（SSE 解析）不属于传输阶段，不得被误升为可重试
        assert_eq!(upstream_code_for_hints("type=decode；sse 解析失败"), None);
    }

    #[test]
    fn adr_0086_regression_classify_retryability() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        for probe in [
            "provider 返回错误状态：522 <unknown status code>；耗时=6401ms；响应正文：",
            "provider 返回错误状态：520 <unknown status code>",
            // 2026-11-12：525 由「排除」改为「纳入」（源站握手瞬时失败，与 522 同组）
            "provider 返回错误状态：525 <unknown status code>；耗时=6097ms；响应正文：",
            "error sending request for url (https://tokens.ponyjob.top/v1/chat/completions)；type=request；elapsed=30996ms",
        ] {
            assert!(policy.classify(probe).is_retryable(), "{probe}");
        }
        for probe in [
            // 526 Invalid SSL Certificate 仍属确定性配置错误 ⇒ Abort
            "provider 返回错误状态：526 <unknown status code>；耗时=6097ms；响应正文：",
            "provider 返回错误状态：400；响应正文：invalid model",
            "provider returned 500",
        ] {
            assert!(!policy.classify(probe).is_retryable(), "{probe}");
        }
    }

    #[test]
    fn stage2_classify_matches_r1_outcomes_via_registry() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        // ①–④ 可重试
        for probe in [
            "timeout",
            "HTTP 429",
            "503 Service Unavailable",
            "connection reset",
        ] {
            assert!(policy.classify(probe).is_retryable(), "{probe}");
        }
        // ⑤⑦ 不可重试（含 401 不区分冻结）
        for probe in ["HTTP 400", "HTTP 401", "HTTP 403", "provider returned 500"] {
            let kind = policy.classify(probe);
            assert!(!kind.is_retryable(), "{probe}");
            assert!(!kind.requires_request_mutation(), "{probe}");
        }
        // ⑥ 需请求变更
        let kind = policy.classify("context too large");
        assert!(kind.requires_request_mutation());
    }

    #[test]
    fn stage2_rate_limit_hints_registry_matches_contract() {
        // R2.4 九词（PA-171 C3-S2：只扩展 usage limit / allowance）：常量即断言。
        assert_eq!(
            RATE_LIMIT_LONG_BACKOFF_HINTS,
            &[
                "429",
                "rate limit",
                "rate_limit",
                "ratelimit",
                "too many requests",
                "tpm",
                "quota",
                "usage limit",
                "allowance"
            ]
        );
        assert!(is_rate_limit_error("HTTP 429 quota exceeded"));
        assert!(is_rate_limit_error("monthly quota exhausted"));
        assert!(is_rate_limit_error(
            "{\"error\":{\"message\":\"You've reached your 5-hour usage limit\"}}"
        ));
        assert!(is_rate_limit_error(
            "Request denied: the model is out of its allowance for this billing window"
        ));
        assert!(!is_rate_limit_error("operation timed out"));
    }

    // ── PA-171 C3：限流/配额分类与冷却窗口（单一出口） ──────────────────────────────

    #[test]
    fn pa171_classify_error_class_follows_frozen_priority() {
        // C3-S1 优先级：quota → rate_limited → timeout → auth → network → bad_request → other。
        assert_eq!(
            classify_error_class("{\"error\":{\"type\":\"exceeded_current_quota_error\"}}"),
            "quota"
        );
        assert_eq!(
            classify_error_class("HTTP 429 quota exceeded for org"),
            "quota",
            "quota 优先级高于 rate_limited（C3-S1 冻结顺序）"
        );
        assert_eq!(classify_error_class("HTTP 429 Too Many Requests"), "rate_limited");
        assert_eq!(
            classify_error_class("调用 provider 失败：operation timed out after 45002ms"),
            "timeout"
        );
        assert_eq!(classify_error_class("provider 返回错误状态：401 Unauthorized"), "auth");
        assert_eq!(classify_error_class("connection reset by peer"), "network");
        assert_eq!(classify_error_class("HTTP 400 Bad Request"), "bad_request");
        assert_eq!(classify_error_class("boom: unexpected local invariant violation"), "other");
        // 冷却类别判据复用同一分类器。
        assert!(is_rate_limit_cooldown_error("HTTP 429 Too Many Requests"));
        assert!(is_rate_limit_cooldown_error("monthly quota exhausted"));
        assert!(!is_rate_limit_cooldown_error("operation timed out"));
        assert!(!is_rate_limit_cooldown_error("HTTP 503 Service Unavailable"));
    }

    #[test]
    fn pa171_retry_after_marker_round_trips_and_drives_window() {
        let base = "provider 返回错误状态：429 Too Many Requests；响应正文：slow down";
        let with_marker = append_retry_after_marker(base, 3600);
        assert!(with_marker.contains(RETRY_AFTER_MARKER));
        assert_eq!(append_retry_after_marker(&with_marker, 60), with_marker, "幂等");
        assert_eq!(parse_retry_after_marker(&with_marker), Some(3600));
        assert_eq!(parse_retry_after_marker(base), None);
        assert_eq!(
            rate_limit_window_ms(&with_marker, (0, 0)),
            Some(3_600_000),
            "Retry-After 后缀是窗口的第一来源"
        );
        assert_eq!(
            rate_limit_recovery_hint(&with_marker).as_deref(),
            Some("预计 3600s 后恢复")
        );
    }

    #[test]
    fn pa171_l3b_retry_after_marker_and_auth_are_body_proof() {
        // A3：正文里的同名 token 既不得短路权威头值写入，也不得被解析成窗口。
        let poisoned = "provider 返回错误状态：429 Too Many Requests；耗时=4031ms；响应正文：{\"note\":\"retry_after=999999\"}";
        let error = append_retry_after_marker(poisoned, 60);
        assert!(
            error.contains("retry_after=60"),
            "权威头值必须写进文本：{error}"
        );
        assert_eq!(parse_retry_after_marker(&error), Some(60));
        assert_eq!(rate_limit_window_ms(&error, (0, 0)), Some(60_000));
        assert_eq!(
            rate_limit_recovery_hint(&error).as_deref(),
            Some("预计 60s 后恢复")
        );
        assert_eq!(
            parse_retry_after_marker("无头；响应正文：retry_after=999999"),
            None,
            "正文段不得参与 marker 解析"
        );

        // A1：auth 只看头部段 + 词边界，不得被耗时数字或正文措辞劫持。
        for probe in [
            "provider 返回错误状态：429 Too Many Requests；耗时=4031ms；响应正文：{}",
            "provider 返回错误状态：429 Too Many Requests；耗时=401ms；响应正文：{}",
            "provider 返回错误状态：429 Too Many Requests；耗时=18ms；响应正文：for this api key",
        ] {
            assert!(!is_auth_error(probe), "429 不得被误判 auth：{probe}");
        }
        // A2 反向钉子：真 auth 仍判 auth（状态行 401/403 与措辞，词边界）。
        assert!(is_auth_error(
            "provider 返回错误状态：401 Unauthorized；响应正文：{\"message\":\"invalid api key\"}"
        ));
        assert!(is_auth_error(
            "provider 返回错误状态：403 Forbidden；耗时=4ms；响应正文：{}"
        ));
        assert!(!is_auth_error("provider 返回错误状态：503 Service Unavailable"));
    }

    #[test]
    fn pa171_l3b_cooldown_active_marker_round_trips() {
        let plain = "provider 处于限流冷却中（quota）";
        assert!(!is_cooldown_active_error(plain));
        let marked = append_cooldown_active_marker(plain);
        assert!(is_cooldown_active_error(&marked));
        assert_eq!(append_cooldown_active_marker(&marked), marked, "幂等");
        assert!(marked.contains(COOLDOWN_ACTIVE_MARKER));
    }

    #[test]
    fn pa171_utc_reset_hint_is_parsed_and_bounded_by_next_occurrence() {
        let text = "{\"error\":{\"type\":\"exceeded_current_quota_error\",\"message\":\"You've reached your 5-hour usage limit, resets at 06:01 UTC\"}}";
        // 05:00 UTC ⇒ 61 分钟；06:01/07:00 UTC（复位时刻已到/已过）⇒ 60s 软窗口，
        // **不得**跨日回绕成 ~24h（否则被 6h 上界截断 ⇒ 连续 6h 全短路）。
        println!(
            "[L3-A UTC] now=05:00 -> {:?}, now=06:01 -> {:?}, now=07:00 -> {:?}",
            rate_limit_window_ms(text, (5, 0)),
            rate_limit_window_ms(text, (6, 1)),
            rate_limit_window_ms(text, (7, 0))
        );
        assert_eq!(rate_limit_window_ms(text, (5, 0)), Some(61 * 60_000));
        assert_eq!(rate_limit_window_ms(text, (6, 1)), Some(60_000));
        assert_eq!(rate_limit_window_ms(text, (7, 0)), Some(60_000));
        assert!(
            rate_limit_window_ms(text, (23, 59)).unwrap_or(u64::MAX) <= 60_000,
            "复位时刻已过必须返回 ≤60s 软窗口，不得 ≥6h"
        );
        assert_eq!(
            rate_limit_recovery_hint(text).as_deref(),
            Some("预计 06:01 UTC 恢复")
        );
        assert_eq!(rate_limit_window_ms("no window marker here", (5, 0)), None);
    }

    /// wave-35/C3:SSE 错误文本把 `响应预览/endpoint`(含**随机端口**)拼进 message
    /// (`provider/openai_sse.rs` 的 `响应预览: {endpoint}`);端口形如 `:42953`/`:50208`
    /// 会命中裸 `contains` 码表,把永久性协议错误误判为 `TransientRetryable` →
    /// 5 次重试耗尽 mock 队列 → 回合相位错误(control_plane 相位断言 flake,p≈2.8%)。
    /// 判定必须只看原因文本,不受 URL/端口数字影响。
    #[test]
    fn stage2_classify_ignores_endpoint_port_digits() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        for port in ["42953", "50208", "40891", "50417", "15029"] {
            let err = format!(
                "解析 provider SSE 流失败: provider 流式返回中未找到 SSE data 事件;响应预览: \
                 http://127.0.0.1:{port}/v1; elapsed=3ms; endpoint=http://127.0.0.1:{port}/v1/chat/completions"
            );
            let kind = policy.classify(&err);
            assert!(
                !kind.is_retryable(),
                "endpoint 端口数字不得影响分类: {err} -> {kind:?}"
            );
            assert!(
                !is_rate_limit_error(&err),
                "端口中含 429 不得触发限流长退避: {err}"
            );
        }
        // 判据未被削弱:原因文本里的真实码仍必须命中。
        for probe in [
            "HTTP 502 Bad Gateway",
            "HTTP 503 Service Unavailable",
            "HTTP 504 Gateway Timeout",
            "HTTP 408 Request Timeout",
            "HTTP 429 Too Many Requests",
        ] {
            assert!(policy.classify(probe).is_retryable(), "{probe}");
        }
    }

    /// wave-35/C3(同源):endpoint 里的主机名/路径措辞也不得参与分类
    /// (否则 `http://timeout-gw.internal:8123/v1/timeout` 会把永久协议错误判成超时可重试)。
    #[test]
    fn stage2_classify_ignores_endpoint_url_wording() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        let err = "解析 provider SSE 流失败: provider 流式返回中未找到 SSE data 事件; \
                   endpoint=http://timeout-gw.internal:8123/v1/timeout";
        assert!(
            !policy.classify(err).is_retryable(),
            "endpoint 措辞不得影响分类: {err}"
        );
        // 判据未被削弱:原因文本里的网络措辞仍必须命中。
        assert!(policy.classify("connection refused; endpoint=http://a.b:8123/v1").is_retryable());
        assert!(policy.classify("dns failure").is_retryable());
    }

    /// wave-35/G 回归(F1+F2):剥离必须 (a) 对**多字节字符紧贴 URL** 安全(不得出现
    /// `byte index N is not a char boundary` panic —— 入参含上游可控文本),(b) 范围最小:
    /// URL 之前紧贴的数字不得被当作 scheme 名吞掉,(c) URL 后无空白直达串尾时其前文本仍参与分类。
    #[test]
    fn stage2_strip_endpoint_fragments_is_byte_safe_and_minimal() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        // (a) 多字节字符**紧贴** `://`(无 ASCII 分隔符):不 panic,端口不得决定分类。
        // 输入的字符宽度覆盖 CJK(3B)/全角冒号(3B)/emoji(4B);均用转义码点写死,
        // 避免「只有带空格的输入」导致回归时(index+1 复活)测试仍恒绿。
        for probe in [
            "访问https://x/v1",
            "访问https://127.0.0.1:42953/v1",
            "错误\u{ff1a}https://x/v1",
            "错误\u{ff1a}https://127.0.0.1:50208/v1",
            "\u{1f512}https://x/v1",
            "\u{1f512}https://127.0.0.1:40891/v1",
            "错误:https://127.0.0.1:42953/v1",
            "请求失败(网络层):https://gw.internal:50208/v1/chat/completions",
        ] {
            let kind = policy.classify(probe);
            assert!(
                !kind.is_retryable(),
                "多字节紧贴 URL 不得 panic 或按端口误判: {probe} -> {kind:?}"
            );
            assert!(!is_rate_limit_error(probe), "{probe}");
        }
        // (b) URL 前紧贴的数字必须保留(基线 TransientRetryable),不得被当 scheme 名吞掉。
        assert!(policy.classify("HTTP 502https://gw/v1").is_retryable());
        assert!(policy.classify("HTTP 503https://gw/v1").is_retryable());
        assert!(policy.classify("429https://gw/v1").is_retryable());
        // (c) URL 后无空白直达串尾:其前原因文本仍参与分类。
        assert!(policy
            .classify("connection reset by peer http://127.0.0.1:50208/v1")
            .is_retryable());
        assert!(!policy
            .classify("响应预览无 SSE data 事件 http://127.0.0.1:50208/v1")
            .is_retryable());
        // (d) 多个 URL 逐个剥离,后续原因文本保留。
        assert!(policy
            .classify("主站失败 http://a.internal:50208/v1 备用 connection refused")
            .is_retryable());
    }

    /// PA-143：码表是裸 `contains` 时，**更长数字串内的片段**会伪造命中
    /// （`elapsed=40832ms` → `408`、`"code":"429001"` → `429`），使重试结论取决于
    /// 与原因无关的数字。判据必须按结构化字段/词边界判定，而不是整段文本子串。
    #[test]
    fn stage2_classify_does_not_match_digits_inside_longer_numbers() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        for probe in [
            "provider stream aborted; elapsed=40832ms",
            "{\"error\":{\"message\":\"invalid request\",\"code\":\"429001\"}}",
            "{\"error\":{\"message\":\"invalid request\",\"code\":\"1502\"}}",
            "{\"error\":{\"message\":\"invalid request\",\"code\":\"1503\"}}",
            "{\"error\":{\"message\":\"invalid request\",\"code\":\"1504\"}}",
        ] {
            let kind = policy.classify(probe);
            assert!(
                !kind.is_retryable(),
                "更长数字串内的片段不得伪造可重试分类: {probe} -> {kind:?}"
            );
            assert!(
                !is_rate_limit_error(probe),
                "更长数字串内的片段不得伪造限流判定: {probe}"
            );
        }
        // 判据未被削弱：既有合法命中必须保留。
        for probe in [
            "HTTP 429 Too Many Requests",
            "status:502",
            "operation timed out",
        ] {
            assert!(
                policy.classify(probe).is_retryable(),
                "既有合法命中不得丢失: {probe}"
            );
        }
    }

    /// PA-144：无 scheme 的 `host:port` 仍把端口数字暴露给码表
    /// （`endpoint=127.0.0.1:42953/v1` → `429`、`endpoint=localhost:50208` → `502`），
    /// 生产可达路径是 provider 把 endpoint 拼进错误正文且 base_url 不校验 scheme。
    /// 端口数字不得改变分类；端口**恰为码值**（`:429` / `:502`）时纯词边界判定仍会漏，
    /// 因此必须按 host:port 结构剥离。
    #[test]
    fn stage2_classify_ignores_schemeless_host_port_digits() {
        let policy = ProviderRetryPolicy::new(BackoffConfig::default());
        for probe in [
            "解析 provider SSE 流失败: 未找到 SSE data 事件; endpoint=127.0.0.1:42953/v1",
            "解析 provider SSE 流失败: 未找到 SSE data 事件; endpoint=localhost:50208",
            "解析 provider SSE 流失败: 未找到 SSE data 事件; endpoint=127.0.0.1:429/v1",
            "解析 provider SSE 流失败: 未找到 SSE data 事件; endpoint=localhost:502",
        ] {
            let kind = policy.classify(probe);
            assert!(
                !kind.is_retryable(),
                "scheme-less host:port 的端口不得污染分类: {probe} -> {kind:?}"
            );
            assert!(
                !is_rate_limit_error(probe),
                "scheme-less host:port 的端口不得触发限流判定: {probe}"
            );
        }
        // 判据未被削弱：真正的原因文本码仍必须命中。
        assert!(policy.classify("HTTP 502 Bad Gateway").is_retryable());
        assert!(policy.classify("HTTP 429 Too Many Requests").is_retryable());
    }
}
