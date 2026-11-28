//! ACP 宿主面命令（薄封装：解析/状态机全在 `pony_agent_core::acp::host`）。

use crate::blocking_helper::BlockingHelper;
use crate::tauri_adapter;
use pony_agent_core::acp::host::AcpAgentSummary;
use pony_agent_core::acp::host::DelegateRequest;
use serde::Serialize;
use std::sync::Arc;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcpDelegateOutcome {
    delegation_id: String,
    status: String,
    output: String,
    duration_ms: u64,
}

/// 列出可用的外部 ACP agent（用户清单 → 内置默认）。
#[tauri::command]
pub(crate) async fn acp_list_agents() -> Result<Vec<AcpAgentSummary>, String> {
    let agents = BlockingHelper::spawn(pony_agent_core::acp::host::list_agents)
        .await?
        .map_err(|error| format!("{}: {error}", error.code()))?;
    Ok(agents)
}

/// 发起一次宿主侧 ACP 委派（**阻塞到本轮结束**，故走阻塞池，不占 IPC 线程）。
///
/// 取消：`delegation_id` 由调用方（前端）分配；委派运行期间可并发调用
/// [`acp_delegate_cancel`] 翻取消令牌。终态按 ADR `2026-11-23-acp-cancel-race-ok-precedence`
/// 的 **Ok 优先** 口径：取消命中且 agent 未产出终态 ⇒ `aborted` / `tool_cancelled`；
/// agent 已同步回 `Ok` 终态 ⇒ 保留该终态。
#[tauri::command]
pub(crate) async fn acp_delegate_start(
    app: AppHandle,
    delegation_id: String,
    agent: String,
    prompt: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    session_id: Option<String>,
) -> Result<AcpDelegateOutcome, String> {
    let sink = Arc::new(tauri_adapter::TauriAcpDelegateSink::new(
        app,
        delegation_id.clone(),
    ));
    let request = DelegateRequest {
        delegation_id: delegation_id.clone(),
        agent,
        prompt,
        cwd,
        timeout_ms,
        host_session_id: session_id,
    };
    let result = BlockingHelper::spawn(move || {
        pony_agent_core::acp::host::run_delegation(request, sink.as_ref())
    })
    .await?;
    Ok(AcpDelegateOutcome {
        delegation_id,
        status: result.status,
        output: result.output,
        duration_ms: result.duration_ms,
    })
}

/// 取消一次进行中的 ACP 委派：返回是否命中某个活跃委派（幂等，只发信号不等终局）。
#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri 命令参数由 JSON 反序列化，必须 DeserializeOwned（&str 不成立），故收 String 后只借用"
)]
#[tauri::command]
pub(crate) fn acp_delegate_cancel(delegation_id: String) -> bool {
    pony_agent_core::acp::host::cancel_delegation(&delegation_id)
}
