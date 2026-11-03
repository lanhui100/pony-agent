//! 工具面「团队工具不得泄漏」冻结契约（wave-20 起，wave-61 迁移）。
//!
//! ## wave-61 迁移说明（Test Agent 按 Lead 冻结裁决执行；旧断言 → 新断言）
//!
//! | 项 | 旧断言（wave-20 ~ wave-42） | 新断言（wave-61） | 理由 / 契约真值源 |
//! |---|---|---|---|
//! | 用例名 | `test_stub_tools_are_no_longer_model_visible` | `t1_team_tools_are_model_visible` | 语义方向反转，名称随之改；断言集合不缩减 |
//! | 10 个团队工具 | `assert!(!names.contains("<tool>"), "<tool> must not be exposed to models")` | `assert!(names.contains("<tool>"), "<tool> must be exposed to models (wave-61 C1)")` | wave-61 C1.1：`tool_exposure_for_name()`（`tools.rs:7341`）必须为这 10 项返回 `ModelVisible`，原先落 `:7381` 的 `_ => Internal` |
//! | 反向防泄漏 | 无（当时 10 项即"泄漏源"） | 新增：registry 中 exposure 非 `ModelVisible` 的内置工具（除 `tool_search` 白名单例外）一律不得出现在可见面 | 迁移后仍须守住「内部工具不得泄漏」的原始立意；真值源 `tools.rs:622-631` 的过滤分支 |
//!
//! ### v1.2 二次迁移（Lead 裁决，L3-P BLOCKING：`workflow` 退回 `Internal`）
//!
//! | 项 | v1 断言（wave-61 C1） | v1.2 断言（当前） | 理由 / 真值源 |
//! |---|---|---|---|
//! | `workflow` | `assert!(names.contains(&"workflow"))` | `assert!(!names.contains(&"workflow"), "workflow 桩 handler 不得进模型面")` | handler 是只回显 `step.id` 并返回 `status:"success"` 的桩（`orchestration.rs:286-297`），暴露即伪造成功；恢复条件＝真实 DAG 引擎 |
//! | 其余 9 项 | 必须暴露 | 必须暴露（不变） | wave-61 C1.1 |
//!
//! 未放宽任何断言：v1 的 10 条正向断言中 9 条保留、1 条按裁决反转为更严格的负向守卫；
//! 反向防泄漏断言保留（`workflow` 退回 `Internal` 后同时受该守卫覆盖）。
//! 同文件 `test_goal_tools_are_canonical_and_model_visible`（wave-61 C4.1 明确不受本波影响）保持原断言不变。

use pony_agent_core::agent::tools::{
    builtin_tools, builtin_turn_tool_contract_views, canonical_tool_name, tool_exposure_for_name,
    ToolExposure,
};

#[test]
fn t1_team_tools_are_model_visible() {
    let views = builtin_turn_tool_contract_views();
    let names: Vec<_> = views.iter().map(|v| v.name.as_str()).collect();

    // wave-61 C1.1（v1.2：9 项；`workflow` 见下方反向守卫）：必须全部进入模型可见面（逐名断言）。
    assert!(names.contains(&"subagent"), "subagent must be exposed to models (wave-61 C1)");
    // v1.2（Lead 裁决）：`workflow` handler 是只回显 step.id 的桩（orchestration.rs:286-297），
    // 暴露即伪造成功，故必须**不得**出现在模型面（恢复条件＝真实 DAG 引擎落地）。
    assert!(
        !names.contains(&"workflow"),
        "workflow stub handler must NOT be exposed to models (wave-61 v1.2 L3-P BLOCKING)"
    );
    assert!(
        names.contains(&"spawn_teammate"),
        "spawn_teammate must be exposed to models (wave-61 C1)"
    );
    assert!(names.contains(&"list_agents"), "list_agents must be exposed to models (wave-61 C1)");
    assert!(names.contains(&"send_message"), "send_message must be exposed to models (wave-61 C1)");
    assert!(
        names.contains(&"interrupt_agent"),
        "interrupt_agent must be exposed to models (wave-61 C1)"
    );
    assert!(names.contains(&"drain_inbox"), "drain_inbox must be exposed to models (wave-61 C1)");
    assert!(
        names.contains(&"team_task_create"),
        "team_task_create must be exposed to models (wave-61 C1)"
    );
    assert!(
        names.contains(&"team_task_update"),
        "team_task_update must be exposed to models (wave-61 C1)"
    );
    assert!(
        names.contains(&"team_task_list"),
        "team_task_list must be exposed to models (wave-61 C1)"
    );

    // 反向防泄漏（迁移后仍须守住的原始立意；`workflow` 退回 Internal 后同样受此守卫约束）：
    // registry 中 exposure 非 `ModelVisible` 的内置工具不得出现在可见面，
    // 唯一例外是 `tool_search` 的既定白名单（`tools.rs:622-631` 注释：默认延迟发现的单一条目）。
    let mut leaks: Vec<String> = Vec::new();
    for definition in builtin_tools() {
        let canonical = canonical_tool_name(definition.name).unwrap_or(definition.name);
        let exposure = tool_exposure_for_name(definition.name);
        if exposure == ToolExposure::ModelVisible || canonical == "tool_search" {
            continue;
        }
        if let Some(view) = views
            .iter()
            .find(|view| view.execution_primitive == canonical)
        {
            leaks.push(format!(
                "内部工具 `{}`（exposure={:?}）泄漏进可见面，product 名 `{}`",
                definition.name, exposure, view.name
            ));
        }
    }
    assert!(
        leaks.is_empty(),
        "wave-61 反向防泄漏失败：非 ModelVisible 的内置工具不得进入模型可见面（`tool_search` 例外）。\
         实际泄漏：{leaks:?}"
    );
}

#[test]
fn test_goal_tools_are_canonical_and_model_visible() {
    assert_eq!(canonical_tool_name("create_goal"), Some("create_goal"));
    assert_eq!(canonical_tool_name("get_goal"), Some("get_goal"));
    assert_eq!(canonical_tool_name("update_goal"), Some("update_goal"));

    let views = builtin_turn_tool_contract_views();
    let names: Vec<_> = views.iter().map(|v| v.name.as_str()).collect();

    assert!(names.contains(&"create_goal"), "create_goal must be exposed to models");
    assert!(names.contains(&"get_goal"), "get_goal must be exposed to models");
    assert!(names.contains(&"update_goal"), "update_goal must be exposed to models");
}
