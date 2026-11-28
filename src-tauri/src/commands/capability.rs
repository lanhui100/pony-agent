//! 能力/MCP/Skill 检查面命令。

use crate::agent::capability_bridge::CapabilitySourceView;
use crate::agent::capability_bridge::CapabilityView;
use crate::agent::capability_bridge::McpSourceSnapshot;
use crate::agent::capability_bridge::SkillDescriptor;
use crate::agent::control_plane::{
    ApplyMcpSourceSnapshotCommand, CapabilityInspectionQuery, CapabilityListQuery,
    CapabilitySourceInspectionQuery, HostControlPlane, SkillInspectionQuery, SkillListQuery,
};
use crate::agent::mcp_config::McpHealthView;
use crate::agent::mcp_config::McpSourceLoadSummary;
use crate::agent::tools::builtin_turn_tool_contract_views;
use crate::agent::tools::ToolDefinitionContractView;
use tauri::State;

#[tauri::command]
pub(crate) fn list_available_tools() -> Vec<ToolDefinitionContractView> {
    builtin_turn_tool_contract_views()
}

#[tauri::command]
pub(crate) fn list_capability_sources(
    control_plane: State<'_, HostControlPlane>,
) -> Vec<CapabilitySourceView> {
    control_plane.list_capability_sources()
}

/// 应用一份 MCP 来源快照（能力注册表的来源元数据入口；ADR 0066 decision 3）。
///
/// 工具身份与执行仍由 `ToolRegistrySnapshot` + `GovernedDispatcher` 决定，
/// 这里只登记「发现到的来源/能力」事实。
#[tauri::command]
pub(crate) fn apply_mcp_source_snapshot(
    snapshot: McpSourceSnapshot,
    control_plane: State<'_, HostControlPlane>,
) -> Result<CapabilitySourceView, String> {
    control_plane.apply_mcp_source_snapshot(ApplyMcpSourceSnapshotCommand { snapshot })
}

/// MCP 通道健康（ADR 0066 decision 3）：连接状态 / 最近错误码 / 工具数。
/// 前端只经 command 读取，不直连 MCP。
#[tauri::command]
pub(crate) fn mcp_health(control_plane: State<'_, HostControlPlane>) -> Vec<McpHealthView> {
    control_plane.mcp_health()
}

/// 按 `.mcp.json`（`<workspace>/.mcp.json` + `$PONY_AGENT_HOME/mcp.json`）加载并连接 MCP server，
/// 注册进治理分派器并安装为进程级通道（模型工具面下一轮生效）。
#[tauri::command]
pub(crate) fn load_mcp_sources(
    workspace_root: Option<String>,
    control_plane: State<'_, HostControlPlane>,
) -> McpSourceLoadSummary {
    control_plane.load_mcp_sources(workspace_root)
}

#[tauri::command]
pub(crate) fn list_capabilities(
    source_id: Option<String>,
    kind: Option<String>,
    control_plane: State<'_, HostControlPlane>,
) -> Vec<CapabilityView> {
    control_plane.list_capabilities(CapabilityListQuery { source_id, kind })
}

#[tauri::command]
pub(crate) fn inspect_capability(
    capability_id: String,
    control_plane: State<'_, HostControlPlane>,
) -> Option<CapabilityView> {
    control_plane.inspect_capability(CapabilityInspectionQuery { capability_id })
}

#[tauri::command]
pub(crate) fn inspect_capability_source(
    source_id: String,
    control_plane: State<'_, HostControlPlane>,
) -> Option<CapabilitySourceView> {
    control_plane.inspect_capability_source(CapabilitySourceInspectionQuery { source_id })
}

#[tauri::command]
pub(crate) fn list_skills(
    source_id: Option<String>,
    control_plane: State<'_, HostControlPlane>,
) -> Vec<SkillDescriptor> {
    control_plane.list_skills(SkillListQuery { source_id })
}

#[tauri::command]
pub(crate) fn inspect_skill(
    skill_id: String,
    control_plane: State<'_, HostControlPlane>,
) -> Option<SkillDescriptor> {
    control_plane.inspect_skill(SkillInspectionQuery { skill_id })
}
