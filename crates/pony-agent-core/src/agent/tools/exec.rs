//! 执行面公共类型与结果构造器：`ToolCall`/`ToolResult`/`ToolOutcome`/`ToolExecutionContext`、
//! `ToolExecutor` 契约与 `error_result` 系列。
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: Option<String>,
    pub name: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ToolPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub status: String,
    pub output: String,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionStatus {
    Ok,
    Error,
    Cancelled,
}

impl ToolExecutionStatus {
    pub fn from_legacy_status(status: &str) -> Self {
        match status {
            "ok" => Self::Ok,
            "aborted" | "cancelled" => Self::Cancelled,
            _ => Self::Error,
        }
    }

    pub fn as_legacy_status(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "aborted",
        }
    }
}


#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolControlKind {
    WaitingUser,
    ApprovalRequired,
    WaitingHost,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolControlOutcome {
    pub kind: ToolControlKind,
    pub request_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutcome {
    pub execution_status: ToolExecutionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_outcome: Option<ToolControlOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ToolResult>,
}

impl ToolOutcome {
    pub fn from_legacy_result(result: ToolResult) -> Self {
        Self {
            execution_status: ToolExecutionStatus::from_legacy_status(&result.status),
            control_outcome: None,
            result: Some(result),
        }
    }

    pub fn pending(control_outcome: ToolControlOutcome) -> Self {
        Self {
            execution_status: ToolExecutionStatus::Ok,
            control_outcome: Some(control_outcome),
            result: None,
        }
    }

    pub fn into_legacy_result(self, tool_name: impl Into<String>) -> ToolResult {
        self.result.unwrap_or_else(|| ToolResult {
            tool_name: tool_name.into(),
            // A pending control request is not a provider-consumable tool result. Until the
            // runtime consumes it and resumes the original call, legacy callers must fail closed.
            status: "error".to_string(),
            output: json!({
                "ok": false,
                "error": {
                    "code": "control_outcome_pending",
                    "kind": "control_outcome_pending",
                    "message": "工具调用尚处于控制请求状态，尚未产生可供 provider 消费的终态结果。"
                }
            })
            .to_string(),
            duration_ms: 0,
        })
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPlanStep {
    pub name: String,
    pub arguments: Value,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPlan {
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub parallel: bool,
    #[serde(default)]
    pub continue_on_error: bool,
    pub steps: Vec<ToolPlanStep>,
}

pub trait ToolExecutor: Send + Sync {
    fn execute(&self, call: &ToolCall) -> ToolResult;

    fn execute_with_context(&self, call: &ToolCall, _context: &ToolExecutionContext) -> ToolResult {
        self.execute(call)
    }

    /// Optional downcast accessor (PA-076 Ask host mediation). Adapters that wrap a concrete
    /// executor with extra surface — e.g. `GovernedToolExecutor` — expose themselves here so the
    /// runtime can reach the shared governed dispatcher and set per-turn invocation context. The
    /// default implementation returns `None`, keeping every existing `ToolExecutor` implementor
    /// source-compatible.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
}

/// 单次工具调用的不可变执行上下文（PA-080 consultant 方案 A）。
/// 显式贯穿工具方法链，取代共享可变状态：并发调用互不串扰、panic 无残留。
#[derive(Clone, Debug, Default)]
pub struct ToolExecutionContext {
    /// 会话级 workspace root（`None` = 无会话上下文，回退构造时默认 root）。
    pub workspace_root: Option<PathBuf>,
    /// 会话级 workspace id（显式透传或由 call.arguments["workspaceId"] 提供）。
    pub workspace_id: Option<String>,
    /// 当前会话 id（wave-1 C5）：`TodoWrite` / `TodoList` / goal 工具参数缺省 session 时回退用它。
    /// 由 runtime / governed dispatch 注入；模型显式给出的 session 参数优先，不被覆盖。
    pub session_id: Option<String>,
    /// T2-1：本次调用的 turn 级治理事实（session/run/turn/workspace/授权模式/审批证据）。
    ///
    /// 由 runtime 在**每次工具调用**处按本 turn 的事实构造、随调用显式传入 —— 取代此前
    /// `GovernedToolExecutor` 上的共享 `Mutex<DispatchContext>` + `set_context`（并发 turn 会
    /// 互相覆盖，把 A 会话的授权事实带进 B 会话的调用）。`None` 表示调用方未提供 turn 事实：
    /// 治理链必须**明确降级**（session/turn 为空），绝不回退到任何全局或线程局部状态。
    pub dispatch_context: Option<crate::agent::dispatcher::DispatchContext>,
    /// wave-62 C3：本 turn 的取消令牌。`Some` ⇒ 子进程类工具（`workspace_run_command`）
    /// 执行期间须按 `turn_cancel_poll_ms`(≤100ms) 轮询，被取消时终止子进程并返回
    /// `status="aborted"`（错误码沿用既有 `tool_cancelled`）。
    pub cancellation: Option<std::sync::Arc<crate::agent::budget::CancellationToken>>,
    /// W2（additive）：本 turn 宿主当前生效的 provider 选择（子代理/队友 provider 继承源）。
    ///
    /// `None` = 调用方未注入（工具层不得自行猜测或回退任何全局 provider 状态）。
    pub provider_selection: Option<crate::agent::config::ResolvedProviderSelection>,
}

/// wave-1 C5：工具参数未显式给 session 时回退到执行上下文注入的会话 id。
pub(super) fn fill_session_from_context(
    session_id: &mut Option<String>,
    context: &ToolExecutionContext,
) {
    let explicit = session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    if !explicit {
        if let Some(context_session) = context.session_id.as_deref().map(str::trim) {
            if !context_session.is_empty() {
                *session_id = Some(context_session.to_string());
            }
        }
    }
}

/// W26：`spill_read` 参数。
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(super) struct SpillReadArgs {
    pub(super) ref_id: String,
    #[serde(default)]
    pub(super) offset: Option<usize>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
}

/// W25：未接线的子代理/团队能力的统一拒绝结果。
///
/// 这 10 个工具当前只有桩实现（合成字符串 + 进程内静态表），若按「成功」回执返回，模型会
/// 误信能力已执行。这里统一返回结构化 `capability_unavailable`，并给出可行动指引。
pub(super) fn capability_unavailable_result(tool_name: &str) -> ToolResult {
    error_result(
        tool_name,
        "capability_unavailable",
        format!(
            "工具 `{}` 所属的子代理/团队运行时尚未接线，该能力当前不可用；请勿据此假设已派生子代理或已产生子任务。",
            tool_name
        ),
        Some(
            "见 docs/audit/agent-core-improvement-plan.md（计划 W29 落地真实子代理与 workflow 引擎）；\
             在此之前请改用现有真实工具（Read/Search/Run/BatchExecute 等）。"
                .to_string(),
        ),
    )
}

/// 契约 B012：`job_*` 工具的错误映射 —— 归属拒绝使用独立错误码 `job_not_owned`
/// （`error.kind` 与 `code` 同值），其余错误沿用各工具既有的兜底码。
///
/// 拒绝文案固定为通用中文，**不含**作业输出/命令行（lib 层的英文 `Err` 前缀不直接外露）。
pub(super) fn job_tool_error(tool_name: &str, fallback_code: &str, error: String) -> ToolResult {
    if error.starts_with(crate::agent::jobs::JOB_NOT_OWNED_CODE) {
        return error_result(
            tool_name,
            crate::agent::jobs::JOB_NOT_OWNED_CODE,
            "该作业不属于当前会话，已拒绝访问。".to_string(),
            None,
        );
    }
    error_result(tool_name, fallback_code, error, None)
}

pub(super) fn error_result(tool_name: &str, code: &str, message: String, hint: Option<String>) -> ToolResult {
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "error".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": tool_name,
            "error": {
                // Stage1 双写（A2）：kind 与 code 并存且值一致。
                "code": code,
                "kind": code,
                "message": message,
                "hint": hint,
            },
            "summary": {
                "text": message
            }
        })),
        duration_ms: 0,
    }
}

pub(crate) fn tool_error_from_output(status: &str, parsed: &Value) -> Option<ToolError> {
    if status == "ok" {
        return None;
    }

    let error = parsed.get("error")?;
    // B5：纯字符串 error 形状保文本：message 含原文，kind 落 unknown 标记
    //（此前丢文本回默认句；码形 A4 要求 kind 保持 ^[a-z][a-z0-9_]{1,63}$，
    // 故 kind 取 "unknown" 而非嵌入原文）。
    if let Some(raw) = error.as_str() {
        return Some(ToolError {
            kind: "unknown".to_string(),
            message: if raw.is_empty() {
                "Tool execution failed.".to_string()
            } else {
                raw.to_string()
            },
            details: None,
            retryable: None,
            source: None,
        });
    }
    Some(ToolError {
        kind: error
            // B4：冲突时 code 获胜（code 优先，唯一行为变更）。
            .get("code")
            .or_else(|| error.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("tool_error")
            .to_string(),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Tool execution failed.")
            .to_string(),
        details: error.get("details").cloned(),
        retryable: error.get("retryable").and_then(Value::as_bool),
        source: error
            .get("source")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

pub(super) fn aborted_result(tool_name: &str, code: &str, message: String) -> ToolResult {
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "aborted".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": tool_name,
            "status": "aborted",
            "error": {
                // Stage1 双写：kind 与 code 并存且值一致。
                "code": code,
                "kind": code,
                "message": message,
                "hint": Option::<String>::None,
            }
        })),
        duration_ms: 0,
    }
}

/// wave-9 P0-2：与 [`error_result`] 同构的错误结果，额外把 `evidence` 对象的键**并入载荷根级**。
///
/// `error_result` 既有语义保持不变（不含证据键）；本 helper 只服务于「错误必须携带已采集物理
/// 证据」的场景（run_command 超时）。`evidence` 的键名由调用方保证与成功路径一致（禁止别名）。
pub(super) fn error_result_with_evidence(
    tool_name: &str,
    code: &str,
    message: String,
    hint: Option<String>,
    evidence: Value,
) -> ToolResult {
    let mut payload = json!({
        "ok": false,
        "tool": tool_name,
        "error": {
            // Stage1 双写（A2）：kind 与 code 并存且值一致。
            "code": code,
            "kind": code,
            "message": message,
            "hint": hint,
        },
        "summary": {
            "text": message
        }
    });
    if let Some(root) = payload.as_object_mut() {
        if let Some(evidence) = evidence.as_object() {
            for (key, value) in evidence {
                root.insert(key.clone(), value.clone());
            }
        }
    }
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "error".to_string(),
        output: json_string(payload),
        duration_ms: 0,
    }
}

pub(super) fn json_string(value: Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

pub(super) fn parse_tool_output(output: &str) -> Value {
    serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()))
}
