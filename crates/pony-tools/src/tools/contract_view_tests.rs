use super::{
    builtin_tools, builtin_turn_tool_contract_views, canonical_tool_name,
    default_permission_facts_for_name, model_visible_tool_name, primitive_dispatch_table,
    product_canonical_tool_name, tool_display_metadata_for_name, tool_exposure_for_name,
    ToolDescriptor, ToolDescriptorSource, ToolExposure,
    ToolHandlerProvenance, ToolIdentity, ToolKind, ToolRegistrySnapshot,
    TOOL_WORKSPACE_PATH_INFO, TOOL_WORKSPACE_RUN_COMMAND,
};
use serde_json::json;

#[test]
fn builtin_tool_contract_views_deduplicate_to_model_surface() {
    let views = builtin_turn_tool_contract_views();
    let names = views
        .iter()
        .map(|view| view.name.as_str())
        .collect::<Vec<_>>();
    let primitives = views
        .iter()
        .map(|view| view.execution_primitive.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "Run",
            "Ask",
            "Read",
            "ReadDocument",
            "List",
            "Search",
            "Glob",
            "WebFetch",
            "WebSearch",
            "MCPResource",
            "ToolSearch",
            "Write",
            "Edit",
            "BatchExecute",
            "Plan",
            "ViewImage",
            // Wave-61 C1：9 项团队工具进入模型可见面（插在 ViewImage 之后、TodoWrite 之前，
            // 与本文件 `characterization_product_surface_...` 的 39 项清单同序；
            // `workflow` 因桩实现保持 Internal，不在可见面）。
            "subagent",
            "spawn_teammate",
            "list_agents",
            "send_message",
            "interrupt_agent",
            "drain_inbox",
            "team_task_create",
            "team_task_update",
            "team_task_list",
            "TodoWrite",
            "TodoList",
            "create_goal",
            "get_goal",
            "update_goal",
            // W37-C1：七件新模型可见工具（present/fuzzy_file_search/git_diff_remote/job_*）。
            "Present",
            "Find",
            "GitDiff",
            "JobStart",
            "JobOutput",
            "JobKill",
            "JobList",
            // W42/PA-149：技能正文按需加载（产品名保持 `skill_load`）。
            "skill_load",
            // B150/P1：ACP 委派（模型面 38 → 39）。
            "acp_delegate"
        ]
    );
    assert_eq!(
        primitives,
        vec![
            "workspace_run_command",
            "ask_user",
            "workspace_gather_context",
            "workspace_read_document",
            "workspace_list_files",
            "workspace_search_text",
            "workspace_glob_files",
            "web_fetch_url",
            "web_search_query",
            "mcp_resource_read",
            "tool_search",
            "workspace_write_file",
            "workspace_edit_file",
            "workspace_batch",
            "plan_control",
            "view_image",
            // Wave-61 C1：9 项团队工具的 primitive 同样插入 view_image 之后
            //（`workflow` 保持 Internal，不在可见面）。
            "subagent",
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
            "create_goal",
            "get_goal",
            "update_goal",
            // W37-C1：七件新 primitive。
            "present",
            "fuzzy_file_search",
            "git_diff_remote",
            "job_start",
            "job_output",
            "job_kill",
            "job_list",
            // W42/PA-149：七件 W37 工具之后的技能正文加载 primitive。
            "skill_load",
            // B150/P1：ACP 委派 primitive（产品名即 primitive 名）。
            "acp_delegate"
        ]
    );
}

#[test]
fn canonical_tool_name_keeps_run_as_product_name_but_allows_internal_run_shell() {
    assert_eq!(canonical_tool_name("Run"), Some(TOOL_WORKSPACE_RUN_COMMAND));
    assert_eq!(model_visible_tool_name(TOOL_WORKSPACE_RUN_COMMAND), "Run");
    assert_eq!(
        product_canonical_tool_name(TOOL_WORKSPACE_RUN_COMMAND),
        "Run"
    );
}

/// ADR 0065 decision 2：`execute_internal` 从 72 行 `match` 改为 fn 指针表后，编译期穷尽性
/// 检查消失，必须由本遍历断言补回——表与 `builtin_tools()` 的 46 个 primitive 必须**双射**
/// （每个恰好一次，无缺无多余），且表内每个 key 都能被 `canonical_tool_name` 归一化回自身
/// （否则该臂永久不可达）。
#[test]
fn primitive_dispatch_table_covers_every_builtin_primitive() {
    let table = primitive_dispatch_table();
    let mut sorted_table = table.clone();
    sorted_table.sort_unstable();
    sorted_table.dedup();
    assert_eq!(
        sorted_table.len(),
        table.len(),
        "primitive dispatch table contains duplicate primitives: {table:?}"
    );

    let mut builtin_primitives = builtin_tools()
        .iter()
        .map(|definition| definition.name)
        .collect::<Vec<_>>();
    builtin_primitives.sort_unstable();

    assert_eq!(
        builtin_primitives.len(),
        46,
        "builtin_tools() 的 primitive 数已变化；分派表必须同步"
    );
    assert_eq!(
        sorted_table, builtin_primitives,
        "分派表必须是 46 个内建 primitive 的双射（缺臂 ⇒ 工具静默变 unsupported_tool）"
    );

    for primitive in table.iter().copied() {
        assert_eq!(
            canonical_tool_name(primitive),
            Some(primitive),
            "分派表 key `{primitive}` 未被 canonical_tool_name 归一化回自身 ⇒ 该臂不可达"
        );
    }
}

#[test]
fn characterization_product_surface_keeps_unique_names_and_json_object_schemas() {
    let tools = builtin_tools();
    let views = builtin_turn_tool_contract_views();
    let names = views
        .iter()
        .map(|view| view.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        tools.len(),
        46,
        "W42/PA-149 新增 skill_load 后内部注册数 45；B150/P1 追加 acp_delegate → 46"
    );
    assert_eq!(
        names.len(),
        39,
        "Wave-61 C1 后模型可见面 38 项 + B150/P1 的 acp_delegate = 39 项"
    );
    assert_eq!(
        names,
        vec![
            "Run",
            "Ask",
            "Read",
            "ReadDocument",
            "List",
            "Search",
            "Glob",
            "WebFetch",
            "WebSearch",
            "MCPResource",
            "ToolSearch",
            "Write",
            "Edit",
            "BatchExecute",
            "Plan",
            "ViewImage",
            // Wave-61 C1：9 项团队工具按 builtin_tools() 定义序插入 ViewImage 之后
            //（product 名 == primitive 小写原名；`workflow` 因桩实现保持 Internal）。
            "subagent",
            "spawn_teammate",
            "list_agents",
            "send_message",
            "interrupt_agent",
            "drain_inbox",
            "team_task_create",
            "team_task_update",
            "team_task_list",
            "TodoWrite",
            "TodoList",
            "create_goal",
            "get_goal",
            "update_goal",
            // W37-C1：七件新模型可见工具。
            "Present",
            "Find",
            "GitDiff",
            "JobStart",
            "JobOutput",
            "JobKill",
            "JobList",
            // W42/PA-149：技能正文按需加载。
            "skill_load",
            // B150/P1：ACP 委派（模型面 38 → 39）。
            "acp_delegate"
        ]
    );
    for view in views {
        assert_eq!(
            view.input_schema
                .get("type")
                .and_then(|value| value.as_str()),
            Some("object")
        );
        assert!(
            view.input_schema.get("properties").is_some(),
            "{} must retain an object property schema",
            view.name
        );
    }
}

#[test]
fn builtin_tool_schemas_require_ui_descriptions() {
    for tool in builtin_tools() {
        let required = tool
            .input_schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .expect("builtin tool schema should declare required arguments");
        assert!(
            required
                .iter()
                .any(|value| value.as_str() == Some("description")),
            "{} should require a user-visible invocation description",
            tool.name
        );
    }
}

#[test]
fn characterization_aliases_resolve_to_the_current_internal_primitives() {
    let aliases = [
        ("Run", "workspace_run_command"),
        ("Ask", "ask_user"),
        ("Read", "workspace_gather_context"),
        ("List", "workspace_list_files"),
        ("Search", "workspace_search_text"),
        ("Glob", "workspace_glob_files"),
        ("WebFetch", "web_fetch_url"),
        ("WebSearch", "web_search_query"),
        ("MCPResource", "mcp_resource_read"),
        ("ToolSearch", "tool_search"),
        ("Write", "workspace_write_file"),
        ("Edit", "workspace_edit_file"),
        ("BatchExecute", "workspace_batch"),
        ("Plan", "plan_control"),
        ("ViewImage", "view_image"),
        ("time.now", "time_now"),
        ("workspace.read_file", "workspace_read_file"),
    ];

    for (alias, primitive) in aliases {
        assert_eq!(canonical_tool_name(alias), Some(primitive), "alias {alias}");
    }
}

#[test]
fn characterization_legacy_ask_and_plan_mappings_are_migration_removal_baselines() {
    // These assertions document the behavior replaced by PA-076/PA-114. Ask is now the
    // first-class `ask_user` primitive (not the legacy echo placeholder), and Plan must
    // stop dispatching batches.
    assert_eq!(canonical_tool_name("Ask"), Some("ask_user"));
    assert_eq!(canonical_tool_name("BatchExecute"), Some("workspace_batch"));
    // PA-076: Plan now resolves to its own state-control handler rather than workspace_batch.
    assert_eq!(canonical_tool_name("Plan"), Some("plan_control"));
}

#[test]
fn characterization_builtin_permission_projection_matches_current_primitive_classes() {
    let cases = [
        (
            "Read",
            ToolKind::Read,
            ToolExposure::ModelVisible,
            Some("workspace.read"),
        ),
        (
            "Write",
            ToolKind::Write,
            ToolExposure::ModelVisible,
            Some("workspace.write"),
        ),
        (
            "Edit",
            ToolKind::Write,
            ToolExposure::ModelVisible,
            Some("workspace.write"),
        ),
        (
            "Run",
            ToolKind::Execute,
            ToolExposure::ModelVisible,
            Some("workspace.execute"),
        ),
        (
            "ToolSearch",
            ToolKind::Search,
            ToolExposure::Deferred,
            Some("capability.discovery"),
        ),
        (
            "Ask",
            ToolKind::Interactive,
            ToolExposure::ModelVisible,
            None,
        ),
        (
            "BatchExecute",
            ToolKind::Composite,
            ToolExposure::ModelVisible,
            Some("workspace.read"),
        ),
    ];

    for (name, kind, exposure, scope) in cases {
        let view = builtin_turn_tool_contract_views()
            .into_iter()
            .find(|view| view.name == name)
            .expect("product tool should exist");
        assert_eq!(view.kind, kind.as_str(), "kind for {name}");
        assert_eq!(view.exposure, exposure.as_str(), "exposure for {name}");
        assert_eq!(
            default_permission_facts_for_name(name)
                .permission_scope
                .as_deref(),
            scope,
            "permission scope for {name}"
        );
    }
}

#[test]
fn builtin_registry_projects_the_existing_product_surface() {
    let registry = ToolRegistrySnapshot::builtin().expect("builtin registry should build");
    let names = registry
        .provider_contract_views()
        .into_iter()
        .map(|view| view.name)
        .collect::<Vec<_>>();

    assert_eq!(registry.snapshot_id, "builtin-tool-catalog-v1");
    assert_eq!(
        names.len(),
        39,
        "Wave-61 C1 的 38 项 + B150/P1 的 acp_delegate = 39 项"
    );
    assert!(registry.resolve("Run").is_some());
    assert!(registry.resolve("workspace.run_command").is_some());
    assert!(registry.resolve("workspace_path_info").is_some());
    assert!(registry.resolve("Present").is_some());
    assert!(registry.resolve("JobList").is_some());
    assert!(registry.resolve("skill_load").is_some());
    // B150/P1：`acp_delegate` 的 descriptor 必须可解析、ModelVisible、Composite 且带中文展示名。
    assert_eq!(
        registry
            .resolve("acp_delegate")
            .map(|descriptor| descriptor.exposure.clone()),
        Some(ToolExposure::ModelVisible),
        "acp_delegate 必须已在模型可见面（B150/P1）"
    );
    assert_eq!(
        registry
            .resolve("acp_delegate")
            .map(|descriptor| descriptor.kind.clone()),
        Some(ToolKind::Composite)
    );
    // Wave-61 C1：10 项团队工具的 descriptor 必须可解析且已 ModelVisible。
    for name in [
        "subagent",
        "spawn_teammate",
        "list_agents",
        "send_message",
        "interrupt_agent",
        "drain_inbox",
        "team_task_create",
        "team_task_update",
        "team_task_list",
    ] {
        assert_eq!(
            registry.resolve(name).map(|descriptor| descriptor.exposure.clone()),
            Some(ToolExposure::ModelVisible),
            "{name} 必须已在模型可见面（Wave-61 C1）"
        );
    }
    // `workflow` 是桩实现（只回显 step.id + status="success"），必须保持 Internal，
    // 不得进模型可见面（诚实性优先；恢复条件＝真实 DAG 引擎落地）。
    assert_eq!(
        registry
            .resolve("workflow")
            .map(|descriptor| descriptor.exposure.clone()),
        Some(ToolExposure::Internal),
        "workflow 桩实现不得进入模型可见面（Wave-61 L3-P 裁决）"
    );
    assert_eq!(
        registry
            .resolve("ToolSearch")
            .map(|descriptor| descriptor.exposure.clone()),
        Some(ToolExposure::Deferred)
    );
}

#[test]
fn builtin_tool_surface_is_the_single_turn_projection_for_provider_and_host_views() {
    let surface = super::builtin_tool_surface();
    let provider = surface
        .provider_contract_views()
        .expect("builtin turn view should project");
    assert_eq!(provider, super::builtin_turn_tool_contract_views());
    assert_eq!(
        surface.model_name_for_primitive(TOOL_WORKSPACE_RUN_COMMAND),
        Some("Run")
    );
    // `workspace_path_info` is the deferred descriptor behind the existing `List` product
    // name. Phase 2 must not rename it; ToolSearch elevation is what exposes it per turn.
    assert_eq!(
        surface.model_name_for_primitive(TOOL_WORKSPACE_PATH_INFO),
        Some("List")
    );
}

#[test]
fn every_model_visible_builtin_tool_has_a_chinese_display_name() {
    // 不变量：模型可见面的每个内建工具都必须有中文展示名，否则工具消息行会回落英文
    // `canonicalToolName`/`toolName`（`WorkspaceTurnItem.vue` 的兜底分支）。
    // 历史事故：Wave-61 团队工具、W20 goal 工具族、PA-112 todo 工具因产品名与
    // `tool_display_metadata_for_name` 的匹配键不一致而全部缺失中文名。
    let missing: Vec<&str> = builtin_tools()
        .into_iter()
        .filter(|definition| tool_exposure_for_name(definition.name) != ToolExposure::Internal)
        .filter(|definition| {
            tool_display_metadata_for_name(definition.name)
                .display_name_zh
                .is_none()
        })
        .map(|definition| definition.name)
        .collect();
    assert!(
        missing.is_empty(),
        "以下模型可见工具缺少中文展示名（工具消息行会回落英文）：{missing:?}"
    );
}

#[test]
fn registry_rejects_ambiguous_aliases() {
    let registry = ToolRegistrySnapshot::builtin().expect("builtin registry should build");
    let mut first = registry.descriptors[0].clone();
    first.aliases = vec!["shared".to_string()];
    let mut duplicate = ToolDescriptor {
        identity: ToolIdentity {
            descriptor_id: "dynamic:duplicate".to_string(),
            model_name: "Duplicate".to_string(),
            canonical_name: "Duplicate".to_string(),
            primitive_name: "duplicate".to_string(),
            source: ToolDescriptorSource::Dynamic,
        },
        aliases: vec!["shared".to_string()],
        description: "duplicate".to_string(),
        input_schema: json!({ "type": "object", "properties": {} }),
        kind: ToolKind::Read,
        exposure: ToolExposure::Internal,
        permission_declaration: Default::default(),
        execution_policy: Default::default(),
        display_metadata: Default::default(),
        handler_provenance: ToolHandlerProvenance {
            handler_kind: "test".to_string(),
            source_id: "test".to_string(),
        },
        source_revision: "test".to_string(),
        composed_descriptor_ids: Vec::new(),
    };
    duplicate.aliases.push("Duplicate".to_string());

    let error = ToolRegistrySnapshot::from_descriptors("test", vec![first, duplicate])
        .expect_err("ambiguous aliases must fail closed");

    assert!(error.contains("ambiguous"));
}

#[test]
fn registry_rejects_reserved_namespace_and_source_provenance_mismatch() {
    let mut external = ToolRegistrySnapshot::builtin()
        .expect("builtin registry should build")
        .descriptors[0]
        .clone();
    external.identity.descriptor_id = "dynamic:bad".to_string();
    external.identity.source = ToolDescriptorSource::Dynamic;
    external.handler_provenance.source_id = "remote-source".to_string();
    external.aliases = vec!["Run".to_string()];
    assert!(
        ToolRegistrySnapshot::from_descriptors("test", vec![external])
            .expect_err("external aliases cannot occupy builtin product namespace")
            .contains("reserved builtin alias")
    );

    let mut builtin = ToolRegistrySnapshot::builtin()
        .expect("builtin registry should build")
        .descriptors[0]
        .clone();
    builtin.handler_provenance.source_id = "wrong-source".to_string();
    assert!(
        ToolRegistrySnapshot::from_descriptors("test", vec![builtin])
            .expect_err("builtin provenance is part of descriptor identity")
            .contains("builtin-tools provenance")
    );

    let mut mcp = ToolRegistrySnapshot::builtin()
        .expect("builtin registry should build")
        .descriptors[0]
        .clone();
    mcp.identity.descriptor_id = "mcp:claimed-source:tool".to_string();
    mcp.identity.source = ToolDescriptorSource::Mcp;
    mcp.handler_provenance.source_id = "actual-source".to_string();
    mcp.aliases = vec!["remote.tool".to_string()];
    assert!(ToolRegistrySnapshot::from_descriptors("test", vec![mcp])
        .expect_err("MCP descriptor identity must bind the provenance source")
        .contains("does not belong to provenance source"));
}

#[test]
fn registry_rejects_composite_cycles_and_replaces_a_source_atomically() {
    let registry = ToolRegistrySnapshot::builtin().expect("builtin registry should build");
    let mut first = registry.descriptors[0].clone();
    first.identity.descriptor_id = "dynamic:one".to_string();
    first.identity.source = ToolDescriptorSource::Dynamic;
    first.handler_provenance.source_id = "dynamic-source".to_string();
    first.aliases = vec!["dynamic.one".to_string()];
    let mut second = first.clone();
    second.identity.descriptor_id = "dynamic:two".to_string();
    second.identity.model_name = "DynamicTwo".to_string();
    second.identity.canonical_name = "DynamicTwo".to_string();
    second.identity.primitive_name = "dynamic_two".to_string();
    second.aliases = vec!["dynamic.two".to_string()];
    first.composed_descriptor_ids = vec!["dynamic:two".to_string()];
    second.composed_descriptor_ids = vec!["dynamic:one".to_string()];
    assert!(
        ToolRegistrySnapshot::from_descriptors("dynamic-v1", vec![first, second])
            .expect_err("composite cycles must fail closed")
            .contains("dependency cycle")
    );

    let mut old = registry.descriptors[0].clone();
    old.identity.descriptor_id = "dynamic:old".to_string();
    old.identity.source = ToolDescriptorSource::Dynamic;
    old.identity.model_name = "DynamicOld".to_string();
    old.identity.canonical_name = "DynamicOld".to_string();
    old.identity.primitive_name = "dynamic_old".to_string();
    old.handler_provenance.source_id = "dynamic-source".to_string();
    old.aliases = vec!["dynamic.old".to_string()];
    let registry = ToolRegistrySnapshot::from_descriptors("dynamic-v1", vec![old])
        .expect("old dynamic source should be valid");
    let mut replacement = registry.descriptors[0].clone();
    replacement.identity.descriptor_id = "dynamic:new".to_string();
    replacement.identity.model_name = "DynamicNew".to_string();
    replacement.identity.canonical_name = "DynamicNew".to_string();
    replacement.identity.primitive_name = "dynamic_new".to_string();
    replacement.aliases = vec!["dynamic.new".to_string()];
    replacement.source_revision = "dynamic-v2".to_string();
    let replaced = registry
        .replace_source("dynamic-source", "dynamic-v2", vec![replacement])
        .expect("source replacement should construct one atomic new snapshot");
    assert!(replaced.resolve("dynamic.old").is_none());
    assert!(replaced.resolve("dynamic.new").is_some());
}
