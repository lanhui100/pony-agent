// turn_event: turn 内过程事实的事件类型定义（事件溯源演进阶段 1，PA-091）。
// 事件是 append-only 事实层，快照/投影是派生视图。设计见
// openspec/changes/turn-event-log/design.md（对抗审核后定稿）。
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider_types::{BuildContextObservation, TokenUsage};
use crate::session_types::AttachmentReference;
use crate::telemetry::{
    CapabilityInvocationRecord, ProviderLatencyKind, ProviderRequestKind,
};

/// 事件 schema 版本：结构变更才 bump，新增事件类型不 bump（借鉴 dsh SESSION_FORMAT_VERSION）。
pub const EVENT_SCHEMA_VERSION: u64 = 1;

/// PA-095：ignorable 事件类型清单——读取时允许跳过的非必需事件类型
/// （`type` 字段精确匹配）。当前为空：所有事件均为必需，未知/坏 payload
/// 一律 fail loud。读取路径（`load_turn_events_checked`）消费该清单：
/// 解析失败的事件若 type 在清单内则跳过，否则标记会话 degraded 并上抛。
pub const IGNORABLE_EVENT_TYPES: &[&str] = &[];

/// turn 结束原因（turn/end 事件的 reason）。
///
/// B219：`Degraded` = 本轮由 provider **本地兜底**收尾（`fallback_reason` 非空）的降级终态。
/// 与 trace 的 `degraded` 字段（工具失败语义）**严格分离**：两者不同源、不得互相复用。
///
/// 读取兼容契约（手写 `Deserialize`，故不 derive）：未知 wire 值一律退化为
/// [`TurnEndReason::Completed`]——旧读取端读新日志**不得炸开**（fail-soft 读）。
/// 写侧仍走 derive 的 `Serialize`（snake_case），写读两向不对称是刻意取舍。
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnEndReason {
    #[default]
    Completed,
    Error,
    Aborted,
    Cancelled,
    /// B219：provider 本地兜底收尾（非正常完成，但不是失败）。
    Degraded,
    /// B217：进程中断（崩溃/退出后由恢复路径补写）——**仅恢复路径**会写。
    ///
    /// 与 [`TurnEndReason::Error`] 严格分离：Error = 本轮真的失败（上游报错/工具链失败），
    /// Interrupted = 本轮没跑完就被进程终止。两者历史文案与续跑判断都不同，不可互借。
    Interrupted,
}

impl TurnEndReason {
    /// 该终态是否「模型已正常收尾」。
    ///
    /// 唯一真源：合成检查点只对非完成态生效，判定不得散落在各投影点。
    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed)
    }
}

impl<'de> Deserialize<'de> for TurnEndReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = String::deserialize(deserializer)?;
        Ok(match wire.as_str() {
            "completed" => Self::Completed,
            "error" => Self::Error,
            "aborted" => Self::Aborted,
            "cancelled" => Self::Cancelled,
            "degraded" => Self::Degraded,
            "interrupted" => Self::Interrupted,
            // 未知 wire 值（旧版本读新日志）⇒ 兼容读为 Completed，不得让读取端炸开。
            _ => Self::Completed,
        })
    }
}

/// B219/P0-3：`turn:completed` 的终态派生口径（唯一真源；写路径与验收测试共用）。
///
/// `fallback_reason` 是本轮 provider 兜底收尾的失败原因：非空 ⇒ 本轮**不是**正常完成，
/// 以 [`TurnEndReason::Interrupted`] 收尾（**可重试/可续跑**的中断态）；`None` / 空白串 ⇒
/// [`TurnEndReason::Completed`]（空白不得伪造降级）。
///
/// P0-3 变更（实测 pa_01a1e908 / pa_01a1e928）：此前非空 ⇒ `Degraded`，而 `Degraded` 在
/// 投影层被当成 `turn.completed` 呈现，于是「上游连续失败、runtime 合成兜底文案」的轮次
/// 在 UI 上与正常完成无法区分——用户看到的就是「毫无征兆地停在中途」。
/// `Degraded` 变体保留（旧日志读取兼容），新写入一律用 `Interrupted`。
pub fn completed_turn_end_reason(fallback_reason: Option<&str>) -> TurnEndReason {
    match fallback_reason.map(str::trim) {
        Some(reason) if !reason.is_empty() => TurnEndReason::Interrupted,
        _ => TurnEndReason::Completed,
    }
}

/// 一条 turn 内过程事件（append-only，落盘后不可变）。
///
/// 事件携带 `branch_id`（阶段 3 起分支可见性推导的基石，阶段 1 默认 "main"）。
/// 构造/反序列化失败 fail loud（数据完整性错误）；持久化写入失败 contained
/// （IO 错误，只记日志不阻断流）——两个失败面由调用方分层处理。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnEvent {
    TurnStart {
        turn_id: String,
    },
    TurnEnd {
        turn_id: String,
        reason: TurnEndReason,
        turn_duration_ms: Option<u64>,
        /// 失败原因（与写路径 `persist_failed_turn_context` 的 `error` 同源）。
        /// additive：旧日志无该键 ⇒ None ⇒ 检查点文本退化为既有中断形态。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// P2-6：结构化停止原因（wire 值见 [`crate::agent::stop_escalation::StopReason`]：
        /// `user` / `parent` / `hook` / `watchdog` / `legacy`）。
        ///
        /// 缺陷：取消轮此前一律写死 `error = "stopped_by_user"`（`runtime/turn_control.rs`），
        /// 实测 pa_01a1e908 那个 108s、0 次 provider 调用的 `cancelled` 轮**无法区分**
        /// 是人工停止还是看门狗收口。additive：旧日志无该键 ⇒ `None`（读取端按 `legacy` 呈现）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stop_reason: Option<String>,
        /// P2-6：失败语义码（`retry::classify_error_class` 口径：`timeout` / `network` /
        /// `rate_limited` / `server_error` …）。`None` ⇒ 非失败轮。
        ///
        /// 缺陷：失败归因此前只藏在自由文本里，读取端只能靠子串猜；有了稳定码，
        /// 「可重试」判定与 UI 文案都不再依赖字符串匹配。additive：旧日志无该键 ⇒ `None`。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_code: Option<String>,
    },
    StepStart {
        turn_id: String,
        step: u32,
        first_token_latency_ms: Option<u64>,
    },
    StepEnd {
        turn_id: String,
        step: u32,
    },
    UserMessage {
        turn_id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<AttachmentReference>,
    },
    AssistantChunk {
        turn_id: String,
        step: u32,
        text: String,
    },
    AssistantMessage {
        turn_id: String,
        step: u32,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        /// 仅消息元数据展示，不参与 MetricsProjection totals（防双计数）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<TokenUsage>,
        /// 回填/降级标记：过程 chunk 不可恢复时置 true。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chunk_missing: Option<bool>,
    },
    ToolCall {
        turn_id: String,
        step: u32,
        call_id: String,
        name: String,
        arguments: String,
        /// TurnToolActivity.duration_seconds 重建所需。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        started_at_ms: Option<u64>,
        /// 工具步骤的人类可读描述（检查点工具行 `name + " " + description` 的载体，
        /// 与 turn_persist 写路径同形）。additive：旧日志无该键 ⇒ None ⇒ 工具行只含 name。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// wave-acp-delegate C1.1：该工具活动所属的**父活动 id**（ACP 子活动归集时
        /// = 触发委派的 `acp_delegate` call_id；顶层工具活动 ⇒ `None`）。
        ///
        /// additive（键缺席 = 顶层活动）：旧日志/旧消费者不读即行为不变。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_activity_id: Option<String>,
    },
    ToolResult {
        turn_id: String,
        step: u32,
        call_id: String,
        /// 32KB 截断在投影层做，事件层存引用或截断标记。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifacts: Option<Vec<Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capability_invocation: Option<CapabilityInvocationRecord>,
        /// wave-acp-delegate C1.1：同 `ToolCall::parent_activity_id`（父活动 call_id；
        /// 顶层工具活动 ⇒ `None`）。additive：旧日志无该键 ⇒ 顶层语义不变。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_activity_id: Option<String>,
    },
    /// **已退役（retired，B1795824704407）**：Plan 收敛为会话级协作模式后
    /// （ADR 2026-11-27-plan-mode-dsh-alignment-and-task-count），Plan 状态一律经控制面
    /// `plan_mode_get` 拉取，本变体**不再有任何 emit 点**。
    ///
    /// 保留而不删除的理由是**持久化兼容**：`TurnEvent` 是会话日志的线缆枚举，删掉变体会让
    /// 含 `plan/update` 的历史日志反序列化失败。故按 additive-retire 处理——变体在、wire 名
    /// 在、投影仍能读历史日志，但新代码**不得再发**该事件。复活条件见
    /// `.agents/notes/implemented/simplification/2026-11-28-retire-plan-update-wire-event.md`。
    PlanUpdate {
        plan_state: Value,
    },
    /// 每次 provider 调用结算（ProviderCallCacheRecord 的事件化）。
    /// 缓存命中（cache_hit/cache_miss）只有 provider 结算时才知道，必须进事件，
    /// 否则 ADR 0007 的缓存命中一等指标无法从事件重建。
    ProviderUsage {
        turn_id: String,
        /// 初始请求=0，每次 tool followup +1（MetricsProjection addReplacing 的唯一键）。
        step: u32,
        request_kind: ProviderRequestKind,
        usage: TokenUsage,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_hit_input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_miss_input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        prefix_mutation_reasons: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        first_token_latency_ms: Option<u64>,
        /// 本次调用耗时（非 turn 耗时；turn 耗时以 turn/end 为准）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_duration_ms: Option<u64>,
        latency_kind: ProviderLatencyKind,
        provider: String,
        model: String,
        /// wave-acp-delegate C5.1（B240）：本 call 的**尝试次数**——`ProviderCallCacheRecord.attempts`
        /// 的事件化载体。
        ///
        /// 缺陷：`provider/usage` 此前只带 usage/耗时，`attempts` / `failed_attempt_ms` /
        /// `prelude_ms` 三键从未进事件流 ⇒ 事件重建（`startup_repair` refold 回写缓存）后
        /// 重试事实静默归零，C5.1 的「仅重试时出现」语义在重启后失效。
        /// additive：旧日志无该键 ⇒ `None`（未知，而非「0 次 / 1 次」）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempts: Option<u32>,
        /// 同 `ProviderCallCacheRecord.failed_attempt_ms`：失败 attempt 的累计耗时（毫秒），
        /// **仅真发生重试时** `Some(>0)`，否则键缺席。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failed_attempt_ms: Option<u64>,
        /// 同 `ProviderCallCacheRecord.prelude_ms`：首个成功 attempt 前的等待/预热（毫秒），
        /// **仅真发生重试时** `Some(>0)`，否则键缺席。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prelude_ms: Option<u64>,
    },
    /// 上下文压缩：投影丢弃 base_seq 之前的消息事件并注入摘要
    /// （折叠不复活被压缩历史）。
    HistorySquash {
        base_seq: u64,
        summary_message: String,
    },
    CheckpointCreated {
        node_id: String,
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_node_id: Option<String>,
    },
    CheckpointCheckout {
        node_id: String,
        mode: String,
    },
    ForkCreated {
        branch_id: String,
        from_node_id: String,
    },
    /// PA-094：大字段外置——build_context_observation 事件化。
    /// 事件只存引用（`observation_ref = "bco:<turn_id>:<seq>"`，seq 为事件日志 seq，
    /// flush 时由 `flush_events_tx` 分配），全量 payload 存独立表
    /// `build_context_observations`。内存构造期携带全量 payload（`observation`，
    /// serde skip 不落盘）；落盘形态只含引用（反序列化后 `observation` 为 None）。
    /// 新增事件类型不 bump `EVENT_SCHEMA_VERSION`（结构变更才 bump）。
    ContextObservation {
        turn_id: String,
        step: u32,
        #[serde(skip)]
        observation: Option<BuildContextObservation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        observation_ref: Option<String>,
    },
    /// 坏事件墓碑节点（PA-096 Phase 2）：遇无法解析的非致命坏事件时，降级为墓碑隔离，
    /// 记录损坏序号与错误摘要，放行其余主干事件投影。
    CorruptedEventTombstone {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        corrupted_seq: u64,
        error_message: String,
        raw_snippet: String,
    },
    /// B214：provider 请求重试（`llm/retry`）——「即将重试」的真实事实落进会话事件流。
    ///
    /// 为什么需要它：实时进度此前只经 `agent::retry` 的 thread_local 观察者投给宿主
    /// （`UiEvent::ProviderRetry`），进程内一次性、不跨界面、重启即丢；排障只能翻运行日志。
    /// 本事件与 DSH 的 `llm/retry` 对齐，把客户端投影「正在重试模型请求（`retry`/`max_attempts`）·
    /// `delay_ms`」所需的字段一次写全。
    ///
    /// 契约（additive）：
    /// - 只在**真实发生**的重试时写（`RetryDecision::Retry` 分支、sleep 之前），不伪造进度；
    /// - `retry` = 已失败尝试次数（1 起，即「第几次重试」）；
    /// - `max_attempts` = 本跳允许的**尝试总数（含首次）**——分母与实时通道同源，
    ///   客户端直接显示 `retry/max_attempts`，无需自行 +1；
    /// - `failure_code` 取自 `retry::classify_error_class` 的冻结取值；不落上游错误正文
    ///   （正文含端点/响应片段，无投影价值且属受控内容）。
    ///
    /// 事件名与 payload `type` 同为 `llm/retry`（与既有 snake_case tag 的差异是本事件
    /// 的显式契约：DSH 同名事件，便于跨端对齐）。
    #[serde(rename = "llm/retry")]
    LlmRetry {
        turn_id: String,
        /// provider 调用名（`decision` / `decision_stream` / `followup` …），与 provider_log 同源。
        request: String,
        /// 重试作用域（`decision` / `followup`）。
        scope: String,
        /// 失败分类（`retry::classify_error_class`：timeout / rate_limited / quota /
        /// network / auth / bad_request / other）。
        failure_code: String,
        /// 已失败尝试次数（1 起）——即「第 retry 次重试」。
        retry: u32,
        /// 本跳允许的尝试总数（含首次）——客户端直接作分母。
        max_attempts: u32,
        /// 本次退避将 sleep 的毫秒数（尚未执行）。
        delay_ms: u64,
    },
    /// P1-4：重试**真正开始**（退避已结束、下一次 attempt 即将发出）的持久化事实。
    ///
    /// 与 [`TurnEvent::LlmRetry`] 成对，构成 dsh 口径的 **durable-before-wait**：
    /// 先落 `llm/retry`（含本次退避时长）→ 退避 → 退避结束后落本事件 → 发 attempt。
    /// 只有 `llm/retry` 时，「进程在退避中被打断」与「重试已发出」在事件层无法区分
    /// （实测 pa_01a1e908 的长轮次复盘即卡在这里：末次 attempt 到底有没有发出去，无从判定）。
    LlmRetryStarted {
        turn_id: String,
        /// provider 调用名（同 [`TurnEvent::LlmRetry`]）。
        request: String,
        /// 重试作用域（`decision` / `followup`）。
        scope: String,
        /// 失败分类（`retry::classify_error_class` 口径）。
        failure_code: String,
        /// 即将发出的 attempt 序号（1 起 = 已失败次数 + 1）。
        attempt: u32,
        /// 本跳允许的尝试总数（含首次）。
        max_attempts: u32,
    },
    /// F1：**轮级速度口径结算**（每轮终态一条，可离线取证）。
    ///
    /// 为什么需要它：`turnSpeedBasis` / speed 此前只走线缆与（401 全丢的）OTel，
    /// 不落 DB、不落日志 ⇒ 「60-70 vs 200+」无法归因（诊断 `.dev-team/diag-tps/00-SYNTHESIS.md` §5.2）。
    /// 本事件把「速度 + 口径 + 样本量 + 突发剔除数」钉进 append-only 事件层，
    /// 重启/离线复盘都能从 `turn_events` 复算，不依赖任何旁路。
    ///
    /// 契约（F1 I1，逐字）：
    /// - 键名按契约**字面量** camelCase（与既有事件 snake_case 约定不同——契约冻结了这五个键名，
    ///   取证脚本/验收断言按名匹配；诊断扫描也是按 camelCase 搜 payload 的）；
    /// - **无值时不写键**（`None` ⇒ 整键缺席，禁止 `0` / `NaN` 占位）；
    /// - 值来源唯一：`speed`/`basis` 与快照同源同帧（`weighted_turn_speed`），
    ///   `turnCallsTotal` / `turnCallsEligible` / `turnBurstExcluded` 由 `turn_flow::turn_call_census`
    ///   从同一份记录派生（判据不在此重写）。
    TurnSpeed {
        turn_id: String,
        #[serde(
            default,
            rename = "turnSpeedTokensPerSecond",
            skip_serializing_if = "Option::is_none"
        )]
        turn_speed_tokens_per_second: Option<f64>,
        #[serde(
            default,
            rename = "turnSpeedBasis",
            skip_serializing_if = "Option::is_none"
        )]
        turn_speed_basis: Option<crate::speed::TurnSpeedBasis>,
        #[serde(default, rename = "turnCallsTotal", skip_serializing_if = "Option::is_none")]
        turn_calls_total: Option<u32>,
        #[serde(
            default,
            rename = "turnCallsEligible",
            skip_serializing_if = "Option::is_none"
        )]
        turn_calls_eligible: Option<u32>,
        #[serde(
            default,
            rename = "turnBurstExcluded",
            skip_serializing_if = "Option::is_none"
        )]
        turn_burst_excluded: Option<u32>,
        /// 契约 I1 原文口径的「全部解码剔除数」（`valid − decode_eligible`）：突发整包、
        /// 无 TTFT、单 token 三类之和。突发只是其中一类，故与 `turnBurstExcluded` 并列保留。
        #[serde(
            default,
            rename = "turnExcludedTotal",
            skip_serializing_if = "Option::is_none"
        )]
        turn_excluded_total: Option<u32>,
        /// `valid` 中**无 TTFT 观测**的调用数（`= valid − 有 ttft 的 valid`）：
        /// 无法界定解码窗口、只能进端到端回退，是「整包到达 / 非流式 followup」的根因面
        /// （诊断 `.dev-team/diag-tps/00-SYNTHESIS.md` §3：73% gemini 调用整包到达）。
        /// 与 `turnBurstExcluded` **互斥不重叠**：突发 = 有 ttft 但 gen 低于物理下限。
        #[serde(
            default,
            rename = "turnCallsTtftMissing",
            skip_serializing_if = "Option::is_none"
        )]
        turn_calls_ttft_missing: Option<u32>,
    },
}

impl TurnEvent {
    /// 事件类型名（`domain/action` 格式，与 `event_type` 列一致）。
    pub fn type_name(&self) -> &'static str {
        match self {
            TurnEvent::TurnStart { .. } => "turn/start",
            TurnEvent::TurnEnd { .. } => "turn/end",
            TurnEvent::StepStart { .. } => "step/start",
            TurnEvent::StepEnd { .. } => "step/end",
            TurnEvent::UserMessage { .. } => "user/message",
            TurnEvent::AssistantChunk { .. } => "assistant/chunk",
            TurnEvent::AssistantMessage { .. } => "assistant/message",
            TurnEvent::ToolCall { .. } => "tool/call",
            TurnEvent::ToolResult { .. } => "tool/result",
            TurnEvent::PlanUpdate { .. } => "plan/update",
            TurnEvent::ProviderUsage { .. } => "provider/usage",
            TurnEvent::HistorySquash { .. } => "history/squash",
            TurnEvent::CheckpointCreated { .. } => "checkpoint/created",
            TurnEvent::CheckpointCheckout { .. } => "checkpoint/checkout",
            TurnEvent::ForkCreated { .. } => "fork/created",
            TurnEvent::ContextObservation { .. } => "context/observation",
            TurnEvent::CorruptedEventTombstone { .. } => "corrupted/tombstone",
            TurnEvent::LlmRetry { .. } => "llm/retry",
            TurnEvent::LlmRetryStarted { .. } => "llm/retry_started",
            TurnEvent::TurnSpeed { .. } => "turn/speed",
        }
    }

    /// 事件归属的 turn（无 turn 语义的事件返回 None）。
    pub fn turn_id(&self) -> Option<&str> {
        match self {
            TurnEvent::TurnStart { turn_id }
            | TurnEvent::TurnEnd { turn_id, .. }
            | TurnEvent::StepStart { turn_id, .. }
            | TurnEvent::StepEnd { turn_id, .. }
            | TurnEvent::UserMessage { turn_id, .. }
            | TurnEvent::AssistantChunk { turn_id, .. }
            | TurnEvent::AssistantMessage { turn_id, .. }
            | TurnEvent::ToolCall { turn_id, .. }
            | TurnEvent::ToolResult { turn_id, .. }
            | TurnEvent::ProviderUsage { turn_id, .. }
            | TurnEvent::LlmRetry { turn_id, .. }
            | TurnEvent::LlmRetryStarted { turn_id, .. }
            | TurnEvent::TurnSpeed { turn_id, .. }
            | TurnEvent::ContextObservation { turn_id, .. } => Some(turn_id),
            TurnEvent::CorruptedEventTombstone { turn_id, .. } => turn_id.as_deref(),
            TurnEvent::PlanUpdate { .. }
            | TurnEvent::HistorySquash { .. }
            | TurnEvent::CheckpointCreated { .. }
            | TurnEvent::CheckpointCheckout { .. }
            | TurnEvent::ForkCreated { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B217：`interrupted` 是新增 wire 值（仅恢复路径写）；
    /// 旧读取端遇未知值仍 fail-soft 退化为 `Completed`，新读取端能认出它。
    #[test]
    fn interrupted_reason_round_trips_and_unknown_wire_stays_fail_soft() {
        let parsed: TurnEndReason =
            serde_json::from_str("\"interrupted\"").expect("interrupted 必须可解析");
        assert_eq!(parsed, TurnEndReason::Interrupted);
        assert_eq!(
            serde_json::to_string(&TurnEndReason::Interrupted).unwrap(),
            "\"interrupted\""
        );

        let unknown: TurnEndReason =
            serde_json::from_str("\"forked\"").expect("未知 wire 值不得炸开");
        assert_eq!(
            unknown,
            TurnEndReason::Completed,
            "旧读取端兼容契约：未知终态退化为 Completed"
        );
    }

    fn sample_usage() -> TokenUsage {
        TokenUsage {
            input_tokens: Some(100),
            cache_hit_input_tokens: Some(40),
            cache_hit_source: None,
            reasoning_tokens: Some(10),
            output_tokens: Some(50),
            total_tokens: Some(160),
            origin: None,
            cache_creation_input_tokens: None,
        }
    }

    #[test]
    fn serde_round_trip_semantic_equivalence() {
        let events: Vec<TurnEvent> = vec![
            TurnEvent::TurnStart {
                turn_id: "t1".into(),
            },
            TurnEvent::TurnEnd {
                turn_id: "t1".into(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: Some(1234),
                error: None,
                stop_reason: None,
                failure_code: None,
            },
            TurnEvent::StepStart {
                turn_id: "t1".into(),
                step: 0,
                first_token_latency_ms: Some(88),
            },
            TurnEvent::StepEnd {
                turn_id: "t1".into(),
                step: 0,
            },
            TurnEvent::UserMessage {
                turn_id: "t1".into(),
                text: "hello".into(),
                attachments: Vec::new(),
            },
            TurnEvent::AssistantChunk {
                turn_id: "t1".into(),
                step: 0,
                text: "hi".into(),
            },
            TurnEvent::AssistantMessage {
                turn_id: "t1".into(),
                step: 0,
                text: "hi".into(),
                reasoning_content: None,
                usage: Some(sample_usage()),
                chunk_missing: None,
            },
            TurnEvent::ToolCall {
                turn_id: "t1".into(),
                step: 1,
                call_id: "c1".into(),
                name: "bash".into(),
                arguments: "{}".into(),
                started_at_ms: Some(1000),
                description: None,
                parent_activity_id: None,
            },
            TurnEvent::ToolResult {
                turn_id: "t1".into(),
                step: 1,
                call_id: "c1".into(),
                result: Some("ok".into()),
                error: None,
                status: Some("done".into()),
                duration_ms: Some(50),
                artifacts: None,
                capability_invocation: None,
                parent_activity_id: None,
            },
            TurnEvent::PlanUpdate {
                plan_state: serde_json::json!({"steps": ["a"]}),
            },
            TurnEvent::ProviderUsage {
                turn_id: "t1".into(),
                step: 0,
                request_kind: ProviderRequestKind::InitialRequest,
                usage: sample_usage(),
                cache_hit_input_tokens: Some(40),
                cache_miss_input_tokens: Some(60),
                prefix_mutation_reasons: Vec::new(),
                first_token_latency_ms: Some(88),
                turn_duration_ms: Some(500),
                latency_kind: ProviderLatencyKind::ProviderStream,
                provider: "deepseek".into(),
                model: "deepseek-chat".into(),
                attempts: None,
                failed_attempt_ms: None,
                prelude_ms: None,
            },
            TurnEvent::HistorySquash {
                base_seq: 10,
                summary_message: "summary".into(),
            },
            TurnEvent::CheckpointCreated {
                node_id: "n1".into(),
                kind: "turn_committed".into(),
                parent_node_id: None,
            },
            TurnEvent::CheckpointCheckout {
                node_id: "n1".into(),
                mode: "transcript_only".into(),
            },
            TurnEvent::ForkCreated {
                branch_id: "b2".into(),
                from_node_id: "n1".into(),
            },
            TurnEvent::ContextObservation {
                turn_id: "t1".into(),
                step: 0,
                observation: None,
                observation_ref: Some("bco:t1:3".into()),
            },
            TurnEvent::LlmRetry {
                turn_id: "t1".into(),
                request: "decision".into(),
                scope: "decision".into(),
                failure_code: "rate_limited".into(),
                retry: 2,
                max_attempts: 5,
                delay_ms: 3_000,
            },
        ];
        for event in &events {
            let json = serde_json::to_string(event).expect("serialize");
            let decoded: TurnEvent = serde_json::from_str(&json).expect("deserialize");
            // 语义等价断言：反序列化后重序列化与原始序列化一致（serde 确定性）。
            assert_eq!(
                serde_json::to_string(&decoded).expect("re-serialize"),
                json,
                "round-trip mismatch for {}",
                event.type_name()
            );
            let re_json = serde_json::to_string(&decoded).expect("re-serialize");
            let re_decoded: TurnEvent = serde_json::from_str(&re_json).expect("re-deserialize");
            assert_eq!(
                serde_json::to_string(&re_decoded).expect("re-re-serialize"),
                re_json,
                "double round-trip mismatch"
            );
        }
    }

    #[test]
    fn type_name_and_turn_id() {
        let start = TurnEvent::TurnStart {
            turn_id: "t1".into(),
        };
        assert_eq!(start.type_name(), "turn/start");
        assert_eq!(start.turn_id(), Some("t1"));
        let squash = TurnEvent::HistorySquash {
            base_seq: 1,
            summary_message: "s".into(),
        };
        assert_eq!(squash.type_name(), "history/squash");
        assert_eq!(squash.turn_id(), None);
    }

    #[test]
    fn unknown_type_fails_loud() {
        let bad = r#"{"type":"alien/event","turn_id":"t1"}"#;
        let result: Result<TurnEvent, _> = serde_json::from_str(bad);
        assert!(result.is_err(), "unknown event type must fail loud");
    }

    #[test]
    fn schema_version_is_stable() {
        assert_eq!(EVENT_SCHEMA_VERSION, 1);
    }

    /// B214：`llm/retry` 的事件名与字段是客户端投影契约（DSH 同名事件）：
    /// `retry`/`max_attempts` 直接给出「2/5」，`delay_ms` 给出退避秒数。
    #[test]
    fn llm_retry_event_exposes_projection_contract() {
        let event = TurnEvent::LlmRetry {
            turn_id: "t-retry".into(),
            request: "followup_stream".into(),
            scope: "followup".into(),
            failure_code: "rate_limited".into(),
            retry: 2,
            max_attempts: 5,
            delay_ms: 3_000,
        };
        assert_eq!(event.type_name(), "llm/retry");
        assert_eq!(event.turn_id(), Some("t-retry"));
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["type"], "llm/retry", "payload tag 与事件名一致");
        assert_eq!(json["retry"], 2);
        assert_eq!(json["max_attempts"], 5);
        assert_eq!(json["delay_ms"], 3_000);
        assert_eq!(json["failure_code"], "rate_limited");
        let decoded: TurnEvent = serde_json::from_value(json).expect("deserialize");
        assert!(matches!(
            decoded,
            TurnEvent::LlmRetry { retry: 2, max_attempts: 5, delay_ms: 3_000, .. }
        ));
    }

    #[test]
    fn context_observation_round_trip_ref_form_and_payload_form() {
        // 落盘形态（引用）：反序列化后 observation 为 None，observation_ref 保留。
        let ref_event = TurnEvent::ContextObservation {
            turn_id: "t1".into(),
            step: 0,
            observation: None,
            observation_ref: Some("bco:t1:3".into()),
        };
        let json = serde_json::to_string(&ref_event).expect("serialize ref form");
        assert!(
            !json.contains("requestFormat"),
            "ref form must not embed payload: {json}"
        );
        let decoded: TurnEvent = serde_json::from_str(&json).expect("deserialize ref form");
        match decoded {
            TurnEvent::ContextObservation {
                observation,
                observation_ref,
                ..
            } => {
                assert!(observation.is_none(), "ref form has no payload");
                assert_eq!(observation_ref.as_deref(), Some("bco:t1:3"));
            }
            _ => panic!("expected ContextObservation"),
        }
        // 内存构造形态（全量 payload）：serde skip 不落盘，重序列化后仍为引用形态。
        let payload_form = TurnEvent::ContextObservation {
            turn_id: "t1".into(),
            step: 0,
            observation: Some(BuildContextObservation {
                request_format: "chat".into(),
                message_count: 2,
                image_count: 0,
                tool_count: 1,
                temperature: 0.7,
                max_output_tokens: 4096,
                stable_prefix_text: "prefix".into(),
                semi_stable_context_text: String::new(),
                volatile_input_text: "input".into(),
                prefix_mutation_reasons: Vec::new(),
                context_refresh_reason: None,
                instruction_scope_sources: Vec::new(),
                conversation_carry_mode: None,
                request_messages_text: "messages".into(),
                tool_definitions_text: "tools".into(),
            }),
            observation_ref: None,
        };
        let payload_json = serde_json::to_string(&payload_form).expect("serialize payload form");
        assert!(
            !payload_json.contains("requestFormat"),
            "payload must be externalized, not embedded: {payload_json}"
        );
        let decoded_payload: TurnEvent =
            serde_json::from_str(&payload_json).expect("deserialize payload form");
        match decoded_payload {
            TurnEvent::ContextObservation {
                observation,
                observation_ref,
                ..
            } => {
                assert!(observation.is_none(), "payload dropped after serialize");
                assert!(observation_ref.is_none(), "no ref assigned yet");
            }
            _ => panic!("expected ContextObservation"),
        }
        // type_name / turn_id 语义
        assert_eq!(ref_event.type_name(), "context/observation");
        assert_eq!(ref_event.turn_id(), Some("t1"));
    }
}
