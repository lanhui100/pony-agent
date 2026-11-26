//! 执行面公共类型与结果构造器：`ToolCall`/`ToolResult`/`ToolOutcome`/`ToolExecutionContext`、
//! `ToolExecutor` 契约与 `error_result` 系列。
use super::*;

pub use pony_protocol::tool_types::{
    aborted_result, error_result, error_result_with_evidence, json_string, parse_tool_output,
    tool_error_from_output, ToolCall, ToolControlKind, ToolControlOutcome, ToolExecutionStatus,
    ToolOutcome, ToolPlan, ToolPlanStep, ToolResult,
};

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
    pub dispatch_context: Option<crate::dispatcher::DispatchContext>,
    /// wave-62 C3：本 turn 的取消令牌。`Some` ⇒ 子进程类工具（`workspace_run_command`）
    /// 执行期间须按 `turn_cancel_poll_ms`(≤100ms) 轮询，被取消时终止子进程并返回
    /// `status="aborted"`（错误码沿用既有 `tool_cancelled`）。
    pub cancellation: Option<std::sync::Arc<pony_protocol::budget::CancellationToken>>,
    /// W2（additive）：本 turn 宿主当前生效的 provider 选择（子代理/队友 provider 继承源）。
    ///
    /// `None` = 调用方未注入（工具层不得自行猜测或回退任何全局 provider 状态）。
    pub provider_selection: Option<pony_provider::config::ResolvedProviderSelection>,
    /// B161：本 turn 的 ACP 权限 handler 句柄（`acp_delegate` 装配给 ACP 会话的 Ask 往返入口）。
    ///
    /// 仅同进程运行时句柄（不序列化）；`None` ⇒ `acp_delegate` 回落 `FailClosedPermissionHandler`
    /// （权限请求一律 cancelled，与 B161 前行为一致）。
    pub acp_permission: Option<std::sync::Arc<dyn pony_protocol::acp_protocol::PermissionHandler>>,
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
pub fn capability_unavailable_result(tool_name: &str) -> ToolResult {
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
    if error.starts_with(crate::jobs::JOB_NOT_OWNED_CODE) {
        return error_result(
            tool_name,
            crate::jobs::JOB_NOT_OWNED_CODE,
            "该作业不属于当前会话，已拒绝访问。".to_string(),
            None,
        );
    }
    error_result(tool_name, fallback_code, error, None)
}
