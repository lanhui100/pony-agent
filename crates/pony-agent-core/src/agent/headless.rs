//! 无头 CLI（`ponycli`）契约面 —— PA-164 / wave-59。
//!
//! 本文件是 headless 交付形态的**生产实现**（wave-59 / T3 阶段）。
//! 契约真值源：`.dev-team/contract-matrix-wave59-headless.md`（v4）。
//!
//! 契约覆盖五类无头执行事实：
//! 1. 结构化事件输出（[`OutputFormat`] / [`JsonlSink`] / [`SseSink`]）；
//! 2. 退出码分类（[`ExitCode`]）；
//! 3. 结果封套（[`ResultEnvelope`]）；
//! 4. 非交互审批策略（[`PermissionMode`] 仅透传宿主 `tool_authorization_mode`，
//!    不逐工具裁决 —— 无策略求值接缝，见类型级说明）；
//! 5. 有界执行（[`HeadlessOptions::timeout_secs`]；工具跳数上限属 PA-167）。
//!
//! 分工：本文件只产出**纯函数 + sink + 有界执行闸门**；真实 turn 的驱动
//! （provider 装配、control plane、结果封套落盘、进程退出码）在 `src/bin/cli.rs`。
//!
//! fail-closed 纪律（契约 C2）：任何未知失败文本一律归 [`ExitCode::Internal`]，
//! **绝不**兜底 [`ExitCode::Success`]；`turn:failed` 永不退 0。

use crate::agent::runtime::TurnStreamEvent;
use crate::agent::turn_flow::TurnEventSink;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// 结果封套的固定 schema 标识（冻结值，不得按波次变更）。
pub const RESULT_ENVELOPE_SCHEMA: &str = "pony.headless.result.v1";

/// 输出形态：纯文本 / JSON Lines / Server-Sent Events。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Jsonl,
    Sse,
}

impl FromStr for OutputFormat {
    type Err = String;

    /// 解析 CLI 传入的 `--output-format` 取值；非法值返回 `Err`。
    ///
    /// 严格白名单、不做大小写或空白归一：非法值必须 `Err`，供 CLI 映射为
    /// [`ExitCode::Usage`]（exit 2），**不得**静默退化为 `text`。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(OutputFormat::Text),
            "jsonl" => Ok(OutputFormat::Jsonl),
            "sse" => Ok(OutputFormat::Sse),
            other => Err(format!(
                "无效的 --output-format 取值 `{other}`（可选：text | jsonl | sse）"
            )),
        }
    }
}

/// 无头执行的输入选项。
///
/// v2（Lead 裁定 D1）：**无** `allow_tools` 字段 —— 宿主不存在可注入的策略求值接缝
/// （`governed_executor.rs:281-287` 的 `governed_default` 兜底 `Allow`，evaluator 由
/// runtime 内部注册于 `governed_executor.rs:463-469`，CLI 无法登记自定义放行判定），
/// 故不做「逐工具白名单」的虚假授权承诺（PA-131 同类裁决）。
///
/// v3（Lead 裁定 A2）：**无** `max_turns` 字段 —— 一次 CLI 调用只跑一轮 turn，该字段
/// 在无头单发场景下恒等于 1（语义空洞）；若要用以表达「工具跳数上限」，运行时只有
/// `runtime/limits.rs:76 tool_hop_limit_override_registry()` 这类 `pub(super)` 内部原子量，
/// 宿主无公开接线口。声明一个无人消费的限制字段＝静默谎报，故该能力连同
/// `--allow-tools` 一并登记为 **PA-167** 另行立项。
#[derive(Debug, Clone)]
pub struct HeadlessOptions {
    pub output_format: OutputFormat,
    pub result_json_path: Option<PathBuf>,
    pub permission_mode: PermissionMode,
    pub timeout_secs: Option<u64>,
}

impl Default for HeadlessOptions {
    /// fail-closed 默认：纯文本输出、不落结果文件、**只读模式**、无超时。
    ///
    /// 只读默认正是本波要修的缺陷 —— 今天 CLI 传 `None` 等于完全不受限。
    fn default() -> Self {
        Self {
            output_format: OutputFormat::Text,
            result_json_path: None,
            permission_mode: PermissionMode::ReadOnly,
            timeout_secs: None,
        }
    }
}

/// 进程退出码分类（数值语义见 [`ExitCode::code`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Success,
    Internal,
    Usage,
    Config,
    ProviderNetwork,
    ToolFailure,
    PermissionDenied,
    Timeout,
}

impl ExitCode {
    /// 退出码数值：Success=0 / Internal=1 / Usage=2 / Config=3 /
    /// ProviderNetwork=4 / ToolFailure=5 / PermissionDenied=6 / Timeout=7。
    pub fn code(self) -> i32 {
        match self {
            ExitCode::Success => 0,
            ExitCode::Internal => 1,
            ExitCode::Usage => 2,
            ExitCode::Config => 3,
            ExitCode::ProviderNetwork => 4,
            ExitCode::ToolFailure => 5,
            ExitCode::PermissionDenied => 6,
            ExitCode::Timeout => 7,
        }
    }

    /// 从失败原因文本归类退出码。
    ///
    /// 事实约束：`TurnStreamEvent.error` 是自由文本（`runtime/types.rs`），`ErrorCode`
    /// 未上 wire ⇒ 只能按文本分档。契约 C2 要求 **fail-closed**：
    /// 空串/未知文本/看似成功的词（`ok` / `success` / `completed`）一律归
    /// [`ExitCode::Internal`]，**永不**返回 [`ExitCode::Success`]。
    ///
    /// 分档顺序即优先级（先命中先返回），依据 `provider/mod.rs` 的真实报错措辞：
    /// 超时 → 权限 → 用法 → 配置 → provider 网络/协议 → 工具。
    pub fn classify_failure(err: &str) -> ExitCode {
        let text = err.to_ascii_lowercase();
        if text.trim().is_empty() {
            return ExitCode::Internal;
        }
        if contains_any(&text, &["timeout", "timed out", "deadline", "超时", "超限", "max_elapsed"]) {
            return ExitCode::Timeout;
        }
        if contains_any(
            &text,
            &[
                "permission denied",
                "permission_denied",
                "not permitted",
                "read_only",
                "read-only",
                "approval required",
                "interaction_unavailable",
                "suspended",
                "审批",
                "授权",
                "只读",
            ],
        ) {
            return ExitCode::PermissionDenied;
        }
        if contains_any(
            &text,
            &[
                "invalid argument",
                "unknown argument",
                "missing required",
                "usage error",
                "参数错误",
                "缺少参数",
            ],
        ) {
            return ExitCode::Usage;
        }
        if contains_any(
            &text,
            &["config", "configuration", "settings", "缺少 api key", "配置文件"],
        ) {
            return ExitCode::Config;
        }
        // 工具边界（v5 B-ADV5：必须**先于** provider 判定。否则 "tool ... http 500"
        // 这类工具失败文本会因含 `http` 被误判成 ProviderNetwork(4)）。
        if contains_any(
            &text,
            &["tool", "工具", "command failed", "exit status", "bash"],
        ) {
            return ExitCode::ToolFailure;
        }
        // provider 边界：连接/协议/流式解析失败（真实措辞见 `provider/mod.rs` 与
        // `provider/openai_sse.rs`：`调用 provider ... 失败`、`type=connect`、
        // `provider 流式返回中未找到 SSE data 事件`、`解析 provider 返回失败` 等）。
        if contains_any(
            &text,
            &[
                "provider",
                "connect",
                "connection",
                "network",
                "socket",
                "dns",
                "tls",
                "handshake",
                "sse",
                "stream",
                "http",
                "原始响应",
                "响应正文",
            ],
        ) {
            return ExitCode::ProviderNetwork;
        }
        ExitCode::Internal
    }
}

/// 大小写不敏感的子串命中（`haystack` 调用方已小写化）。
fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

/// 非交互审批策略（v2）。
///
/// 映射到宿主既有 `tool_authorization_mode` 三档（真值源
/// `crates/pony-agent-core/src/agent/governed_executor.rs:310-333`）：
/// * `read_only` —— **唯一有实际分支**的一档：写入/执行类工具（`ToolKind::Write`
///   / `ToolKind::Execute` 及 primitive/model 名命中 write|edit|bash|execute）被判 `Deny`；
/// * `workspace_write` / `full_access` —— **已知局限**：宿主当前对这两个取值
///   与 `None` 判定等价（无独立分支），故本档位为**声明性**传递，不额外收紧。
///
/// v2（Lead 裁定 D1）删除了 v1 的 `decide()` —— 无头侧无法自行裁决单次工具调用
/// （无策略求值接缝），只做模式透传，严禁在上层伪造放行结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::WorkspaceWrite
    }
}

impl FromStr for PermissionMode {
    type Err = String;

    /// 解析 CLI 传入的 `--permission-mode` 取值。
    ///
    /// 接受：`deny` / `read-only` / `read_only` → [`PermissionMode::ReadOnly`]；
    /// `workspace-write` / `workspace_write` → [`PermissionMode::WorkspaceWrite`]；
    /// `full-access` / `full_access` → [`PermissionMode::FullAccess`]；其余一律 `Err`（fail-closed）。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "deny" | "read-only" | "read_only" => Ok(PermissionMode::ReadOnly),
            "workspace-write" | "workspace_write" => Ok(PermissionMode::WorkspaceWrite),
            "full-access" | "full_access" => Ok(PermissionMode::FullAccess),
            other => Err(format!(
                "无效的 --permission-mode 取值 `{other}`（可选：deny | read-only | read_only | \
                 workspace-write | workspace_write | full-access | full_access）"
            )),
        }
    }
}

impl PermissionMode {
    /// 映射为宿主 `tool_authorization_mode` 的字符串取值：
    /// `"read_only"` / `"workspace_write"` / `"full_access"`。
    ///
    /// 真值源：`governed_executor.rs:310-333`（仅 `read_only` 有实际分支，见类型级说明）。
    pub fn tool_authorization_mode(self) -> &'static str {
        match self {
            PermissionMode::ReadOnly => "read_only",
            PermissionMode::WorkspaceWrite => "workspace_write",
            PermissionMode::FullAccess => "full_access",
        }
    }
}

/// 有界执行的运行结局（[`run_with_deadline`] 的返回值，sink 终态查询同此类型）。
///
/// 语义边界（Lead 冻结）：超时只返回 [`HeadlessOutcome::Timeout`]，
/// **不 panic、不强杀 worker 线程**（worker 侧资源回收由 runner 自身负责）。
///
/// v2（Lead 裁定 D2）：新增 [`HeadlessOutcome::Suspended`] —— 审批/ask 触发时 turn
/// 立即返回且只发 `turn:suspended`（`agent/runtime/turn_stream.rs:376-438`），控制面
/// 亦把 suspended 当**终态**记账（`control_plane/mod.rs:1956-1975`）。无头宿主没有
/// 任何审批应答方，故必须按终态收敛（fail-closed），严禁等待/死循环。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadlessOutcome {
    Completed,
    Failed,
    Timeout,
    Cancelled,
    Suspended,
}

/// 轮次终态（写入 [`ResultEnvelope::status`]）。
///
/// 退出码映射表（Lead 冻结，由 [`exit_code_for`] 表达）：
///
/// | `TurnStatus` | 退出码 | 说明 |
/// | --- | --- | --- |
/// | `Completed` | `ExitCode::Success`(0) | 正常完成 |
/// | `Failed` | `ExitCode::Internal`(1) | 本函数无失败文本入参，故保守归 Internal；真实进程码由 CLI 用 `classify_failure(err)` 得出 |
/// | `Suspended` | `ExitCode::PermissionDenied`(6) | 审批/ask 挂起；无头无应答方 ⇒ fail-closed |
/// | `Timeout` | `ExitCode::Timeout`(7) | 有界执行超时 |
/// | `Cancelled` | `ExitCode::Internal`(1) | 无头下不可达（无交互取消源），保守归 Internal |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnStatus {
    Completed,
    Failed,
    Timeout,
    Cancelled,
    Suspended,
}

/// 终态 → 退出码的纯映射（无 IO、无状态）。
///
/// 注意 `Failed`：本函数签名不携带失败文本，无法按 C2 分档，故保守返回
/// [`ExitCode::Internal`]（fail-closed，绝不 Success）。真实进程退出码由调用方
/// 以 `ExitCode::classify_failure(失败原文)` 求得，二者同源（同一 i32 变量）。
pub fn exit_code_for(status: TurnStatus) -> ExitCode {
    match status {
        TurnStatus::Completed => ExitCode::Success,
        TurnStatus::Failed => ExitCode::Internal,
        TurnStatus::Timeout => ExitCode::Timeout,
        TurnStatus::Cancelled => ExitCode::Internal,
        TurnStatus::Suspended => ExitCode::PermissionDenied,
    }
}

/// token 用量摘要。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct UsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// 单次工具调用摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolCallSummary {
    pub name: String,
    pub status: String,
}

/// 无头执行结果封套（写入 stdout / `result_json_path`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResultEnvelope {
    pub schema: String,
    pub session_id: String,
    pub turn_id: String,
    pub status: TurnStatus,
    pub exit_code: i32,
    pub final_text: String,
    pub usage: UsageSummary,
    pub tool_calls: Vec<ToolCallSummary>,
    pub duration_ms: u64,
    /// v4（裁定 B3）：失败/挂起的 wire 原文（如 `control_outcome_pending` 等），
    /// 透传不加工 —— 使 `suspended` / permission 类原因可辨识的最小手段；无则 `None`。
    ///
    /// `Serialize` 未加 `skip_serializing_if`：键**恒定**出现在 wire 上，无失败时为 `null`。
    pub failure_reason: Option<String>,
}

/// sink 缓冲容量上限（NFR `max_queue_capacity_1024` / `bounded_queue`）。
///
/// 溢出策略（v5 A-ADV，二者取一并写明）：**丢最旧、保留最近 1024 条**；首次溢出向
/// stderr 打一行诊断，绝不静默无限增长。理由：超时后 worker 不可抢占，仍可能持续
/// 投递事件，保留最近事件对排障更有价值。
const MAX_BUFFERED_EVENTS: usize = 1024;

/// sink 的私有缓冲状态：已产出的行 + 事件序号 + 已观测终态。
///
/// `emit` 在 worker 线程被调用 ⇒ 由 `Mutex` 串行化，故 `JsonlSink` / `SseSink`
/// 天然 `Send + Sync`（契约 C1）。
#[derive(Debug, Default)]
struct SinkBuffer {
    lines: Vec<String>,
    /// 单调递增事件序号（从 1 开始），落在 JSONL 每行的 `seq` 字段上。
    seq: u64,
    /// 已观测到的轮次终态；只有终态事件会写它（fail-closed：过程事件不提前收工）。
    outcome: Option<HeadlessOutcome>,
    /// 因容量上限被丢弃的最旧事件数（诊断/取证用）。
    dropped: u64,
    /// 溢出诊断是否已上报（每个 sink 只报一次，避免刷屏）。
    overflow_reported: bool,
}

impl SinkBuffer {
    /// 写入一行，并施加 `MAX_BUFFERED_EVENTS` 上限（丢最旧 + 首次 stderr 诊断）。
    fn push_line(&mut self, make_line: impl FnOnce(u64) -> String) {
        self.seq += 1;
        let seq = self.seq;
        let line = make_line(seq);
        self.push_bounded(line);
    }

    /// 写入一条已格式化好的行（SSE 帧走这里：帧自带 id，无需 sink 侧序号）。
    fn push_bounded(&mut self, line: String) {
        while self.lines.len() >= MAX_BUFFERED_EVENTS {
            let _ = self.lines.remove(0);
            self.dropped += 1;
        }
        if self.dropped > 0 && !self.overflow_reported {
            self.overflow_reported = true;
            eprintln!(
                "[pony-headless] WARN sink 缓冲达上限 {MAX_BUFFERED_EVENTS} 条（max_queue_capacity_1024），\
                 已丢弃最旧事件 dropped={}；机器消费方将看不到这些早先事件",
                self.dropped
            );
        }
        self.lines.push(line);
    }

    /// 归纳终态事件（`turn:completed` / `turn:failed` / `turn:cancelled` /
    /// `turn:suspended`）；其它事件一律不改终态。
    fn observe_terminal(&mut self, name: &str, payload: &TurnStreamEvent) {
        // name 与 payload.kind 在真实 runtime 中同源（`emit("turn:xxx", payload.kind="turn:xxx")`）；
        // name 为空时才回落到 payload.kind（测试夹具两者一致，任一实现都得到同一结论）。
        let event_name = if name.is_empty() {
            payload.kind.as_str()
        } else {
            name
        };
        let outcome = match event_name {
            "turn:completed" => Some(HeadlessOutcome::Completed),
            "turn:failed" => Some(HeadlessOutcome::Failed),
            "turn:cancelled" => Some(HeadlessOutcome::Cancelled),
            "turn:suspended" => Some(HeadlessOutcome::Suspended),
            _ => None,
        };
        if let Some(outcome) = outcome {
            self.outcome = Some(outcome);
        }
    }
}

fn lock(buffer: &Mutex<SinkBuffer>) -> MutexGuard<'_, SinkBuffer> {
    buffer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 把事件序列化成单行 JSON：`{seq, type, ...payload}`（payload 字段平铺在根级）。
///
/// 契约 C1：每行必须是 JSON 对象且含数值型 `seq` 与字符串型 `type`（事件名），
/// 且**不得含 ANSI 转义**（`\x1b`）—— 故对全部字符串字段做 ANSI 剥离。
fn event_json(name: &str, payload: &TurnStreamEvent, seq: u64) -> String {
    let mut record = match sanitize_value(serde_json::to_value(payload).unwrap_or(Value::Null)) {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("payload".to_string(), other);
            map
        }
    };
    // seq / type 为 sink 自有的封套字段，优先级高于 payload 同名字段。
    record.insert("seq".to_string(), Value::from(seq));
    record.insert(
        "type".to_string(),
        Value::String(strip_ansi(name)),
    );
    Value::Object(record).to_string()
}

/// 递归剥离 JSON 所有字符串值/键里的 ANSI 转义序列。
fn sanitize_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(strip_ansi(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(sanitize_value).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (strip_ansi(&key), sanitize_value(value)))
                .collect(),
        ),
        other => other,
    }
}

/// 剥离 ANSI 转义序列（CSI 与单字符 ESC 序列），保留其余可见文本。
///
/// 依据：契约 C1 —— jsonl/sse 的 stdout 是机器消费面，模型文本/工具输出可能夹带
/// 终端控制序列，管道/CI 消费者不得收到它们。
fn strip_ansi(input: &str) -> String {
    // 快速路径：既无 ESC 也无 8-bit CSI 引导符时原样返回（本函数被递归调用，量不小）。
    if !input.contains('\u{1b}') && !input.contains('\u{9b}') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        // v5 B-ADV2：8-bit CSI（U+009B）与 `ESC [ ` 同义，同样吞到终止字节 0x40..=0x7E。
        if ch == '\u{9b}' {
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
            continue;
        }
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            // CSI：ESC '[' 参数/中间字节 + 终止字节（0x40..=0x7E）
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            // 其它转义序列：ESC + 单个字符（如 ESC ']' 的 OSC 头，剩余文本照常保留）
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

/// JSON Lines 事件 sink：每个事件一行 JSON 对象。
///
/// 契约要点（Lead 冻结）：
/// - **全事件透传**：不得按事件名白名单过滤（`started` / `delta` / `tool` / `trace` /
///   `hop_complete` / `output_end` / `phase_changed` / `checkpoint_persisted` /
///   `completed` / `failed` / `cancelled` / `suspended` 等一律成行）；
/// - `emit` 会被 **worker 线程**调用 ⇒ 实现必须 `Send + Sync`（内部用 `Mutex` 缓冲）。
///
/// v4（裁定 B7）：由 unit struct 改为**带私有状态的具名结构体**；字段私有，
/// 外部只能经 [`JsonlSink::new`] 构造，故内部表示可自由演进而不破契约。
#[derive(Debug, Default)]
pub struct JsonlSink {
    /// 私有 sink 状态（缓冲行 + 序号 + 已观测终态）；`emit` 在 worker 线程被调用。
    state: Mutex<SinkBuffer>,
}

impl JsonlSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// 取出已缓冲的输出行（一次性写入或测试查询用的**快照副本**）。
    ///
    /// 缓冲释放语义（NFR `wave59_sink_buffer_released_by_process_exit_only`）：sink 缓冲
    /// **随进程退出释放**，本方法不做中途回收（只返回副本）—— 超时后 worker 不可抢占，
    /// 仍可能继续投递事件；容量由 `max_queue_capacity_1024` 约束（溢出丢最旧 + stderr 诊断）。
    pub fn finish(&self) -> Vec<String> {
        lock(&self.state).lines.clone()
    }

    /// 已观测到的轮次终态（未终态时为 `None`）。
    pub fn outcome(&self) -> Option<HeadlessOutcome> {
        lock(&self.state).outcome
    }
}

impl TurnEventSink for JsonlSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        let mut buffer = lock(&self.state);
        buffer.push_line(|seq| event_json(name, &payload, seq));
        buffer.observe_terminal(name, &payload);
    }
}

/// SSE 事件 sink：按 `event:/id:/data:` 帧格式输出。
///
/// 契约要点同 [`JsonlSink`]：全事件透传 + `Send + Sync`（`Mutex` 缓冲）。
///
/// v4（裁定 B7）：由 unit struct 改为**带私有状态的具名结构体**；字段私有，
/// 外部只能经 [`SseSink::new`] 构造。
#[derive(Debug, Default)]
pub struct SseSink {
    /// 私有 sink 状态（缓冲帧 + 序号 + 已观测终态）；`emit` 在 worker 线程被调用。
    state: Mutex<SinkBuffer>,
}

impl SseSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// 取出已缓冲的 SSE 帧（一次性写入或测试查询用的**快照副本**）。
    ///
    /// 语义同 [`JsonlSink::finish`]：缓冲仅随进程退出释放（NFR
    /// `wave59_sink_buffer_released_by_process_exit_only`），本方法只返回副本。
    pub fn finish(&self) -> Vec<String> {
        lock(&self.state).lines.clone()
    }

    /// 已观测到的轮次终态（未终态时为 `None`）。
    pub fn outcome(&self) -> Option<HeadlessOutcome> {
        lock(&self.state).outcome
    }
}

impl TurnEventSink for SseSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        // 契约 C1 原文：sse 帧格式**复用** `crate::agent::sse_adapter::format_sse_event`
        // （`event:` / `id:` / 逐行 `data:` / 空行终止符），不得自造格式。
        // 序列化失败（`TurnStreamEvent` 全字段可序列化，理论不可达）退化为一行最小帧，
        // 绝不静默吞事件。
        let frame = match crate::agent::sse_adapter::format_sse_event(name, &payload) {
            Ok(frame) => frame,
            Err(_) => format!(
                "event: {}\nid: {}\ndata: {}\n\n",
                strip_ansi(name),
                strip_ansi(&payload.turn_id),
                event_json(name, &payload, 0)
            ),
        };
        let mut buffer = lock(&self.state);
        buffer.push_bounded(strip_ansi(&frame));
        buffer.observe_terminal(name, &payload);
    }
}

/// 有界执行闸门：在 `timeout` 内跑完 `runner` 后返回其结局；
/// 超时返回 [`HeadlessOutcome::Timeout`]（不 panic、不杀 worker）。
///
/// 事实约束（Lead 冻结）：`ControlPlane::start_turn_stream` 是**同步阻塞**的
/// （整轮在调用线程跑完才返回），因此有界执行必须由本函数在**独立 worker 线程**
/// 上运行 runner，调用线程只做带超时的 join —— 不得靠同线程轮询赌时序。
///
/// 结局语义（v4 B9）：
/// * runner 正常返回 ⇒ [`HeadlessOutcome::Completed`]（**是否真的完成由 sink 的终态
///   事件判定**；调用方不得把本返回值直接当 Completed 语义）；
/// * runner panic ⇒ 被 `catch_unwind` 捕获 ⇒ [`HeadlessOutcome::Failed`]（绝不 0）；
/// * 超时 ⇒ [`HeadlessOutcome::Timeout`]，**worker 线程不可抢占**：本函数返回后它可能
///   仍在校验/写 session/trace，资源回收由 runner 自身负责（声明式收敛，非杀线程）。
pub fn run_with_deadline<F>(timeout: Option<Duration>, runner: F) -> HeadlessOutcome
where
    F: FnOnce() + Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::channel::<bool>();
    let spawned = std::thread::Builder::new()
        .name("pony-headless-turn".to_string())
        .spawn(move || {
            // panic 不得外溢成「结局未定义」：统一收敛为 Failed（v4 B9）。
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(runner));
            // 接收端可能已因超时放弃：发送失败静默丢弃即可（无 panic）。
            let _ = sender.send(result.is_ok());
        });
    if spawned.is_err() {
        // 线程创建失败同样 fail-closed，绝不报 Completed。
        return HeadlessOutcome::Failed;
    }

    match timeout {
        None => match receiver.recv() {
            Ok(true) => HeadlessOutcome::Completed,
            // worker 报 Failed 或未发信号即消失 ⇒ fail-closed。
            Ok(false) | Err(_) => HeadlessOutcome::Failed,
        },
        Some(budget) => match receiver.recv_timeout(budget) {
            Ok(true) => HeadlessOutcome::Completed,
            Ok(false) => HeadlessOutcome::Failed,
            Err(RecvTimeoutError::Timeout) => HeadlessOutcome::Timeout,
            Err(RecvTimeoutError::Disconnected) => HeadlessOutcome::Failed,
        },
    }
}
