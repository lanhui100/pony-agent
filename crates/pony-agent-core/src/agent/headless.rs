//! 无头 CLI（`ponycli`）契约面 —— PA-164 / wave-59。
//!
//! 本文件是**纯契约空桩**：只声明 Lead 已冻结的类型、枚举、常量与方法签名，
//! 所有函数体为 `unimplemented!()`。业务实现由 Executor 在后续任务中填充，
//! 验收测试由 Test Agent 依本契约编写（红相按断言失败）。
//!
//! 契约覆盖五类无头执行事实：
//! 1. 结构化事件输出（[`OutputFormat`] / [`JsonlSink`] / [`SseSink`]）；
//! 2. 退出码分类（[`ExitCode`]）；
//! 3. 结果封套（[`ResultEnvelope`]）；
//! 4. 非交互审批策略（[`PermissionMode`] 仅透传宿主 `tool_authorization_mode`，
//!    不逐工具裁决 —— 无策略求值接缝，见类型级说明）；
//! 5. 有界执行（[`HeadlessOptions::timeout_secs`]；工具跳数上限属 PA-167）。
//!
//! 纪律：本文件不得包含分支计算、字符串处理、IO 或任何默认值业务逻辑。

use crate::agent::runtime::TurnStreamEvent;
use crate::agent::turn_flow::TurnEventSink;
use serde::Serialize;
use std::path::PathBuf;
use std::str::FromStr;
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
    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        unimplemented!()
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
        unimplemented!()
    }

    /// 从失败原因文本归类退出码。
    pub fn classify_failure(_err: &str) -> ExitCode {
        unimplemented!()
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

impl FromStr for PermissionMode {
    type Err = String;

    /// 解析 CLI 传入的 `--permission-mode` 取值。
    ///
    /// 接受：`deny` / `read-only` / `read_only` → [`PermissionMode::ReadOnly`]；
    /// `workspace-write` / `workspace_write` → [`PermissionMode::WorkspaceWrite`]；
    /// `full-access` / `full_access` → [`PermissionMode::FullAccess`]；其余一律 `Err`（fail-closed）。
    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        unimplemented!()
    }
}

impl PermissionMode {
    /// 映射为宿主 `tool_authorization_mode` 的字符串取值：
    /// `"read_only"` / `"workspace_write"` / `"full_access"`。
    ///
    /// 真值源：`governed_executor.rs:310-333`（仅 `read_only` 有实际分支，见类型级说明）。
    pub fn tool_authorization_mode(self) -> &'static str {
        unimplemented!()
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
/// | `Failed` | `ExitCode::classify_failure(err)` | 按失败文本分档，未知归 `Internal`(1) |
/// | `Suspended` | `ExitCode::PermissionDenied`(6) | 审批/ask 挂起；无头无应答方 ⇒ fail-closed |
/// | `Timeout` | `ExitCode::Timeout`(7) | 有界执行超时 |
/// | `Cancelled` | `ExitCode::Internal`(1) | 无头下不可达（无交互取消源），保守归 Internal |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TurnStatus {
    Completed,
    Failed,
    Timeout,
    Cancelled,
    Suspended,
}

/// 终态 → 退出码的纯映射（无 IO、无状态）。
pub fn exit_code_for(_status: TurnStatus) -> ExitCode {
    unimplemented!()
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
}

/// JSON Lines 事件 sink：每个事件一行 JSON 对象。
///
/// 契约要点（Lead 冻结）：
/// - **全事件透传**：不得按事件名白名单过滤（`started` / `delta` / `tool` / `trace` /
///   `hop_complete` / `output_end` / `phase_changed` / `checkpoint_persisted` /
///   `completed` / `failed` / `cancelled` / `suspended` 等一律成行）；
/// - `emit` 会被 **worker 线程**调用 ⇒ 实现必须 `Send + Sync`（内部用 `Mutex` 缓冲）。
#[derive(Debug, Default)]
pub struct JsonlSink;

impl JsonlSink {
    pub fn new() -> Self {
        unimplemented!()
    }

    /// 取出已缓冲的输出行（供一次性写入或测试查询）。
    pub fn finish(&self) -> Vec<String> {
        unimplemented!()
    }

    /// 已观测到的轮次终态（未终态时为 `None`）。
    pub fn outcome(&self) -> Option<HeadlessOutcome> {
        unimplemented!()
    }
}

impl TurnEventSink for JsonlSink {
    fn emit(&self, _name: &str, _payload: TurnStreamEvent) {
        unimplemented!()
    }
}

/// SSE 事件 sink：按 `event:/id:/data:` 帧格式输出。
///
/// 契约要点同 [`JsonlSink`]：全事件透传 + `Send + Sync`（`Mutex` 缓冲）。
#[derive(Debug, Default)]
pub struct SseSink;

impl SseSink {
    pub fn new() -> Self {
        unimplemented!()
    }

    /// 取出已缓冲的 SSE 帧（供一次性写入或测试查询）。
    pub fn finish(&self) -> Vec<String> {
        unimplemented!()
    }

    /// 已观测到的轮次终态（未终态时为 `None`）。
    pub fn outcome(&self) -> Option<HeadlessOutcome> {
        unimplemented!()
    }
}

impl TurnEventSink for SseSink {
    fn emit(&self, _name: &str, _payload: TurnStreamEvent) {
        unimplemented!()
    }
}

/// 有界执行闸门：在 `timeout` 内跑完 `runner` 后返回其结局；
/// 超时返回 [`HeadlessOutcome::Timeout`]（不 panic、不杀 worker）。
///
/// 事实约束（Lead 冻结）：`ControlPlane::start_turn_stream` 是**同步阻塞**的
/// （整轮在调用线程跑完才返回），因此有界执行必须由本函数在**独立 worker 线程**
/// 上运行 runner，调用线程只做带超时的 join —— 不得靠同线程轮询赌时序。
pub fn run_with_deadline<F>(_timeout: Option<Duration>, _runner: F) -> HeadlessOutcome
where
    F: FnOnce() + Send + 'static,
{
    unimplemented!()
}

/// 无头执行编排入口：按 [`HeadlessOptions`] 驱动一轮执行并返回退出码。
pub fn run_headless(_opts: &HeadlessOptions) -> ExitCode {
    unimplemented!()
}
