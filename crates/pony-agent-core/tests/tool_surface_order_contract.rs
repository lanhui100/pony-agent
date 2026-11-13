//! 黑盒契约测试：模型工具面（tools 数组）的**顺序与幂等**（Tool Surface Order Contract）
//!
//! 为什么需要它：provider 侧的前缀缓存把 **tools 数组**也算在前缀里（OpenAI 家族的自动缓存按
//! 请求体前缀匹配）。工具面顺序一旦随注册顺序/加载顺序漂移，缓存就会静默失效——请求依旧成功，
//! 只是每轮都按冷缓存计费。仓库里此前没有任何断言保护这个顺序（wave37 只查工具是否存在）。
//!
//! 断言 ID（TS1–TS3）：
//! - TS1：内建工具面的顺序逐项冻结（新增/删除/调整顺序必须显式改本表并说明理由）；
//! - TS2：同一进程内重复取面必须逐项一致（进程级缓存幂等），且无重复名；
//! - TS3：外部工具（MCP/插件通道）必须**追加在内建面之后**，不得插入内建序列中间。
//!
//! 判定真源：`agent::mcp_config::surface_tool_definitions()`。

use pony_agent_core::agent::mcp_config::surface_tool_definitions;
use pony_agent_core::agent::tools::builtin_tools;

/// 内建工具面的冻结顺序（primitive 名，按 `builtin_tools()` 声明顺序）。
const FROZEN_BUILTIN_SURFACE_ORDER: &[&str] = &[
    "time_now",
    "echo_input",
    "ask_user",
    "workspace_read_file",
    "workspace_read_document",
    "workspace_read_file_segment",
    "workspace_list_files",
    "workspace_path_info",
    "workspace_search_text",
    "workspace_glob_files",
    "web_fetch_url",
    "web_search_query",
    "mcp_resource_read",
    "tool_search",
    "workspace_write_file",
    "workspace_edit_file",
    "workspace_run_command",
    "workspace_batch",
    "workspace_gather_context",
    "plan_control",
    "view_image",
    "subagent",
    "workflow",
    "spawn_teammate",
    "list_agents",
    "send_message",
    "interrupt_agent",
    "drain_inbox",
    "team_task_create",
    "team_task_update",
    "team_task_list",
    "todo_write",
    "todo_list",
    "spill_read",
    "create_goal",
    "get_goal",
    "update_goal",
    "present",
    "fuzzy_file_search",
    "git_diff_remote",
    "job_start",
    "job_output",
    "job_kill",
    "job_list",
    "skill_load",
];

/// 取当前工具面的 primitive 名序列。
fn surface_names() -> Vec<String> {
    surface_tool_definitions()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect()
}

/// TS1：内建工具面顺序冻结。
#[test]
fn ts1_builtin_tool_surface_order_is_frozen() {
    let names = surface_names();
    assert_eq!(
        names.len(),
        FROZEN_BUILTIN_SURFACE_ORDER.len(),
        "TS1 违约：工具面条目数变化（{} → {}）。新增/删除工具必须显式更新本表：         tools 数组属于 provider 前缀缓存的一部分，静默变动会让缓存命中率无声下滑",
        FROZEN_BUILTIN_SURFACE_ORDER.len(),
        names.len()
    );
    for (index, expected) in FROZEN_BUILTIN_SURFACE_ORDER.iter().enumerate() {
        assert_eq!(
            names[index], *expected,
            "TS1 违约：第 {index} 个工具由 `{expected}` 变成 `{}`（顺序或条目被改动）",
            names[index]
        );
    }
}

/// TS2：重复取面必须逐项一致（缓存幂等）且无重复名。
#[test]
fn ts2_tool_surface_is_idempotent_and_unique() {
    let first = surface_names();
    let second = surface_names();
    assert_eq!(first, second, "TS2 违约：同一进程内两次取面顺序不一致（缓存不稳定）");

    let mut seen = std::collections::BTreeSet::new();
    for name in &first {
        assert!(
            seen.insert(name.clone()),
            "TS2 违约：工具面出现重复名 `{name}`（重复声明会放大请求体并让工具选择歧义）"
        );
    }
}

/// TS3：外部工具只能追加在内建面之后。
#[test]
fn ts3_external_tools_append_after_the_builtin_prefix() {
    let builtin: Vec<String> = builtin_tools()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    let names = surface_names();
    assert!(
        names.len() >= builtin.len(),
        "TS3 违约：工具面小于内建面（{} < {}），内建工具被丢弃",
        names.len(),
        builtin.len()
    );
    assert_eq!(
        &names[..builtin.len()],
        builtin.as_slice(),
        "TS3 违约：内建工具前缀被外部工具插入或重排（外部工具必须追加在尾部，         否则连接一次 MCP 就会重排整个 tools 数组、打穿前缀缓存）"
    );
}
