use std::time::Duration;

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
/// ③ 5xx/408 四码 → `UpstreamServerError`；④ 连接三词 → `UpstreamNetwork`
/// （禁扩 `connect` 泛词）；⑤ 400 系七码 → `UpstreamBadRequest`/`UpstreamAuth`
/// （401 与 400 不区分冻结，同落 NonRetryable）；⑥ context 四词 → `UpstreamContextOverflow`；
/// ⑦ 其余默认 Abort；⑧ decide/budget/RetryAfter/StreamState 语义冻结（本 Stage 不动）。
///
/// R2.4 七词（`is_rate_limit_error` 冻结）：429/rate limit/rate_limit/ratelimit/
/// too many requests/tpm/quota → `UpstreamRateLimited` 候选（quota-only 因 classify
/// 先判 NonRetryable 而 Abort，quota+429 混合才切 RL 长退避；PA-100 残余原样保留）。
pub(crate) const UPSTREAM_TIMEOUT_HINTS: &[&str] = &[
    "type=timeout",
    "timeout",
    "timed out",
    "deadline has elapsed",
];
pub(crate) const UPSTREAM_RATE_LIMITED_HINTS: &[&str] =
    &["429", "rate limit", "rate_limit", "too many requests", "tpm"];
pub(crate) const UPSTREAM_SERVER_ERROR_HINTS: &[&str] = &["502", "503", "504", "408"];
pub(crate) const UPSTREAM_NETWORK_HINTS: &[&str] =
    &["connection reset", "connection refused", "dns"];
pub(crate) const UPSTREAM_BAD_REQUEST_HINTS: &[&str] =
    &["400", "401", "403", "404", "422", "407", "413"];
pub(crate) const UPSTREAM_CONTEXT_OVERFLOW_HINTS: &[&str] = &[
    "context too large",
    "context_length",
    "max_tokens",
    "payload too large",
];
pub(crate) const UPSTREAM_AUTH_HINTS: &[&str] = &["401", "403"];
pub(crate) const RATE_LIMIT_LONG_BACKOFF_HINTS: &[&str] = &[
    "429",
    "rate limit",
    "rate_limit",
    "ratelimit",
    "too many requests",
    "tpm",
    "quota",
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
    if hints_match(lower, UPSTREAM_SERVER_ERROR_HINTS) {
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
            Some(ErrorCode::UpstreamAuth)
            | Some(ErrorCode::UpstreamBadRequest)
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

/// PA-100：rate-limit 特征判定（单一来源）。`classify` 仍把这类错误统一归为
/// `TransientRetryable`（可重试）；本函数供重试调度方进一步区分"应使用长退避
/// 覆盖分钟级限流窗口"的子类，避免在调用侧各自维护关键词列表造成漂移。
///
/// 消费前提（PA-100 code-review A P2-2；T2-B F2 已明确混合策略）：
/// 仅当 `classify` 先判为可重试时本判定才会被调度方消费——
/// - quota-only（报文仅含 quota/tpm 等措辞，不含 "429"/"rate limit"/timeout 特征）：
///   `classify` 落默认 NonRetryable 分支直接 Abort，不烧退避；
/// - quota+429 混合（报文同时含 quota 与 "429"/"rate limit" 特征）：
///   `classify` 先判 TransientRetryable，本判定再命中 → Followup 下切换 RL 长退避。
///   已接受残余：若底层实为永久配额耗尽而非瞬时限流，会白烧约 60s（20s+40s）退避。
///
/// 判定优先于措辞细节：报文同时含 timeout 与 rate-limit 特征时（如网关把上游
/// 429 包装成超时文案），按 rate-limit 处理——长退避对两类瞬时故障都安全，
/// 反向（短退避撞限流窗口）则必然失败。
pub fn is_rate_limit_error(err: &str) -> bool {
    // R2.4 七词冻结：唯一来源 RATE_LIMIT_LONG_BACKOFF_HINTS。
    let lower = err.to_ascii_lowercase();
    // wave-35/C3: same as classify -- endpoint/URL fragments (ports can contain 429)
    // must not drive the decision. R2.4 wording table itself is unchanged.
    hints_match(&strip_endpoint_fragments(&lower), RATE_LIMIT_LONG_BACKOFF_HINTS)
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
    fn stage2_rate_limit_hints_are_frozen_seven() {
        // R2.4 七词冻结：常量即断言。
        assert_eq!(
            RATE_LIMIT_LONG_BACKOFF_HINTS,
            &[
                "429",
                "rate limit",
                "rate_limit",
                "ratelimit",
                "too many requests",
                "tpm",
                "quota"
            ]
        );
        assert!(is_rate_limit_error("HTTP 429 quota exceeded"));
        assert!(is_rate_limit_error("monthly quota exhausted"));
        assert!(!is_rate_limit_error("operation timed out"));
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
