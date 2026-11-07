//! T1 验收契约（ADR 0065）：工具面单一真值 + 外部工具通道 + 原子换入 + 只读闸门。
//!
//! ## 红相声明（README for reviewers）
//!
//! 本文件由**独立 Test Agent** 在实现落地前写成。约定：Executor 只读本文件，不得修改任何断言。
//!
//! | 用例 | 冻结的契约（ADR 0065） | 落地前期望 |
//! |---|---|---|
//! | `t1_builtin_surface_identity_is_frozen` | 后果「45 个内建工具名称/顺序/schema 逐字节不变」 | 绿（不变量守卫） |
//! | `t1_all_45_builtin_primitives_reach_a_handler` | 决策 2「分派表穷尽覆盖 45 primitive」 | 绿（不变量守卫） |
//! | `t1_unknown_tool_name_is_unsupported` | 决策 9「`unsupported_tool` 仅表示名字确实未知」 | 绿 |
//! | `t1_descriptor_without_handler_fails_closed` | 决策 2「handler 表是唯一分派入口」 | 绿 |
//! | `t1_read_only_denies_external_write_tool_by_declared_permission` | 决策 5「读权限事实，名称不参与判定」 | **红（按名嗅探漏洞回归）** |
//! | `t1_read_only_allows_external_read_tool` | 决策 5 正向对照（不得变成一刀切拒绝） | 绿 |
//! | `t1_external_tool_dispatches_through_governed_pipeline` | 后果「外部 provider 经授权/预算/生命周期返回结果」 | 红（需 `register_tool_provider`） |
//! | `t1_external_provider_error_code_keeps_its_source` | 决策 9「错误码分源」 | 红（同上） |
//! | `t1_external_provider_output_budget_is_enforced` | 后果「预算预约」 | 红（同上） |
//! | `t1_swap_tool_surface_exposes_and_executes_together` | 决策 3「快照 + handler 表原子换入」 | 红（需 `swap_tool_surface`） |
//! | `t1_swap_never_leaves_a_visible_but_unexecutable_descriptor` | 决策 3「不存在可见但不可执行的中间态」 | 红（同上） |
//! | `t1_swap_failure_keeps_the_previous_surface` | 决策 3「原子性：失败不得半换入」 | 红（同上） |
//! | `t1_swap_is_atomic_under_concurrent_dispatch` | 决策 3「任一时刻不存在中间态」 | 红（同上） |
//! | `t1_external_and_plugin_sources_share_one_tool_provider_channel` | 决策 6「只允许一条外部通道」+ Lead 裁决 a（两套 ToolProvider 收敛） | 绿（实现落地后补测） |
//! | `t1_external_and_plugin_failures_keep_their_own_source_families` | 决策 9「错误码分源」跨通道一致性 | 绿（实现落地后补测） |
//! | `t1_external_model_names_resolve_through_the_registry_alias_index` | 决策 7（Lead 裁决）：外部 descriptor 必须把 model_name 登记为 alias | 绿 |
//! | `[KV-CACHE-PREFIX] t1_appending_one_external_tool_keeps_the_builtin_provider_view_byte_identical` | 决策 1「前缀稳定」+ 决策 4「拆除按数组内容推断集合身份」 | 红（`surface_contract_views` 尚未公开 / `is_builtin_tool_definition_set` 仍按内容推断） |
//!
//! ## 冻结的 API 表面（Lead 裁决：test-agent 冻结，executor 只读）
//!
//! ```text
//! tools::{ToolProvider, ToolProviderError::{new, code, message}}
//! GovernedDispatcher::register_tool_provider(&self, Arc<dyn ToolProvider>) -> Result<(), String>
//! GovernedDispatcher::swap_tool_surface(&self, snapshot_id: &str,
//!                                       registry: ToolRegistrySnapshot,
//!                                       handlers: Vec<(String, Arc<dyn PrimitiveToolHandler>)>)
//!                                      -> Result<(), String>
//! ```
//!
//! ## 断言纪律
//!
//! - 负例逐例独立 `#[test]`，每例单独给出可辨识的失败原因，不做整批笼统断言；
//! - 只断言**行为**（派发结果 / 错误码 / 可见性），不断言实现细节：不引用 `PRIMITIVE_DISPATCH_TABLE`
//!   的顺序（Lead 裁决：表顺序是实现自由度），也不引用 `tools.rs` 的任何私有符号；
//! - 45 工具的 golden 是**本文件内的独立字面量**（不是从被测代码推导出来的）。schema 用 FNV-1a
//!   对 `serde_json::to_string(input_schema)` 取值，避免 30KB schema 文本淹没断言。

use pony_agent_core::agent::budget::DispatchBudgetConfig;
use pony_agent_core::agent::dispatcher::{
    DispatchContext, DispatchLifecycleObserver, DispatchLifecyclePhase, DispatchLifecycleRecord,
    GovernedDispatcher, PermissionVerdict,
};
use pony_agent_core::agent::governed_executor::{
    build_governed_executor, GovernedPolicyEvaluator,
};
use pony_agent_core::agent::provider::surface_contract_views;
use pony_agent_core::agent::plugin_host::{
    PluginHostConfig, PluginHostProvider, PluginManifest, PluginSupervisor,
};
use pony_agent_core::agent::tool_runtime::{
    FakeClock, InvocationOrigin, PrimitiveToolHandler, PrimitiveToolHandlerRequest,
    ToolDispatchRequest,
};
use pony_agent_core::agent::tools::{
    builtin_tools, ToolCall, ToolDefinition, ToolDescriptor, ToolDescriptorSource,
    ToolDisplayMetadata,
    ToolExecutionPolicy, ToolExecutionStatus, ToolExposure, ToolHandlerProvenance,
    ToolIdentity, ToolKind, ToolOutcome, ToolPermissionDeclaration, ToolPermissionScope,
    ToolProvider, ToolProviderError, ToolRegistrySnapshot, ToolRouter,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Harness
// ─────────────────────────────────────────────────────────────────────────────────────────────

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 每个用例独占的临时根；`Drop` 时整树删除（谁创建谁清理）。
struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(tag: &str) -> Self {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "pony-t1-contract-{tag}-{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create t1 contract workspace");
        Self { path }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

fn outcome_error_code(outcome: &ToolOutcome) -> Option<String> {
    outcome
        .result
        .as_ref()
        .and_then(|result| serde_json::from_str::<Value>(&result.output).ok())
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn outcome_output(outcome: &ToolOutcome) -> String {
    outcome
        .result
        .as_ref()
        .map(|result| result.output.clone())
        .unwrap_or_default()
}

fn request(
    origin: InvocationOrigin,
    descriptor_id: &str,
    call_id: &str,
    arguments: Value,
) -> ToolDispatchRequest {
    ToolDispatchRequest {
        origin,
        descriptor_id: descriptor_id.to_string(),
        call_id: call_id.to_string(),
        arguments,
    }
}

#[derive(Default)]
struct RecordingObserver {
    records: Mutex<Vec<DispatchLifecycleRecord>>,
}

impl RecordingObserver {
    fn records(&self) -> Vec<DispatchLifecycleRecord> {
        self.records
            .lock()
            .expect("observer mutex poisoned")
            .clone()
    }
}

impl DispatchLifecycleObserver for RecordingObserver {
    fn record(&self, event: &DispatchLifecycleRecord) {
        self.records
            .lock()
            .expect("observer mutex poisoned")
            .push(event.clone());
    }
}

/// 一个最小的合成外部 descriptor（ADR 0065 决策 6：外部来源身份由 `source_id` 承载）。
fn external_descriptor(
    source_id: &str,
    tool: &str,
    kind: ToolKind,
    scopes: &[ToolPermissionScope],
) -> ToolDescriptor {
    let model_name = format!("mcp__{source_id}__{tool}");
    ToolDescriptor {
        identity: ToolIdentity {
            descriptor_id: format!("mcp:{source_id}:{tool}"),
            model_name: model_name.clone(),
            canonical_name: tool.to_string(),
            primitive_name: tool.to_string(),
            source: ToolDescriptorSource::Mcp,
        },
        aliases: vec![model_name],
        description: format!("synthetic external tool `{tool}` from source `{source_id}`"),
        input_schema: json!({ "type": "object", "properties": {} }),
        kind,
        exposure: ToolExposure::ModelVisible,
        permission_declaration: ToolPermissionDeclaration {
            scopes: scopes.iter().cloned().collect::<BTreeSet<_>>(),
            requires_approval: false,
            host_mediated: false,
            approval_mode: None,
        },
        execution_policy: ToolExecutionPolicy::default(),
        display_metadata: ToolDisplayMetadata::default(),
        handler_provenance: ToolHandlerProvenance {
            handler_kind: "external".to_string(),
            source_id: source_id.to_string(),
        },
        source_revision: "external-contract-v1".to_string(),
        composed_descriptor_ids: Vec::new(),
    }
}

fn builtin_snapshot() -> ToolRegistrySnapshot {
    ToolRegistrySnapshot::builtin().expect("builtin registry must validate")
}

fn registry_with(snapshot_id: &str, extra: Vec<ToolDescriptor>) -> ToolRegistrySnapshot {
    let mut descriptors = builtin_snapshot().descriptors;
    descriptors.extend(extra);
    ToolRegistrySnapshot::from_descriptors(snapshot_id, descriptors)
        .expect("test registry must validate")
}

/// 回显真实入参的 handler（证明 handler 收到的是本次调用的参数，而不是测试自造的值）。
struct EchoHandler;

impl PrimitiveToolHandler for EchoHandler {
    fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        Ok(request.arguments.clone())
    }
}

struct CountingHandler(Arc<AtomicUsize>);

impl PrimitiveToolHandler for CountingHandler {
    fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(request.arguments.clone())
    }
}

#[derive(Clone)]
enum ProviderBehavior {
    Return(String),
    Fail(ToolProviderError),
}

/// 测试专用策略评估器：一律 Allow。
///
/// 用于把「两条外部通道是否走同一条治理管线」与「策略面如何判」解耦：
/// 插件工具按实现声明 `requires_approval=true`（保守取值），策略面会要求审批；
/// 那些语义由专门的授权用例覆盖，这里只要通道等价性。
struct AllowAllEvaluator;

impl pony_agent_core::agent::dispatcher::ToolPolicyEvaluator for AllowAllEvaluator {
    fn evaluate(
        &self,
        _descriptor: &ToolDescriptor,
        _origin: &InvocationOrigin,
        _final_arguments: &Value,
    ) -> pony_agent_core::agent::dispatcher::PermissionDecision {
        pony_agent_core::agent::dispatcher::PermissionDecision {
            verdict: PermissionVerdict::Allow,
            decision_source: "test_allow_all".to_string(),
            reason: None,
        }
    }
}

/// 合成一个外部 `ToolProvider`（ADR 0065 决策 6：唯一外部通道），并记录调用点。
struct RecordingProvider {
    source_id: String,
    descriptors: Vec<ToolDescriptor>,
    behavior: ProviderBehavior,
    calls: Mutex<Vec<(String, Value)>>,
}

impl RecordingProvider {
    fn new(source_id: &str, descriptors: Vec<ToolDescriptor>, behavior: ProviderBehavior) -> Self {
        Self {
            source_id: source_id.to_string(),
            descriptors,
            behavior,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().expect("provider calls poisoned").clone()
    }
}

impl ToolProvider for RecordingProvider {
    fn source_id(&self) -> &str {
        &self.source_id
    }

    fn list_tools(&self) -> Vec<ToolDescriptor> {
        self.descriptors.clone()
    }

    fn call(&self, descriptor_id: &str, arguments: Value) -> Result<Value, ToolProviderError> {
        self.calls
            .lock()
            .expect("provider calls poisoned")
            .push((descriptor_id.to_string(), arguments));
        match &self.behavior {
            ProviderBehavior::Return(payload) => Ok(json!({
                "source": self.source_id,
                "payload": payload,
                "marker": "EXTERNAL_PROVIDER_MARKER",
            })),
            ProviderBehavior::Fail(error) => Err(error.clone()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 1. 45 内建工具面冻结（名称 / 顺序 / schema / kind / exposure）
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// `(descriptor_id, model_name, kind, exposure, FNV-1a(serde_json(input_schema)))`，
/// 顺序 = registry 顺序（ADR 0065：registry 位置是每次投影的唯一真值源）。
const BUILTIN_GOLDEN: &[(&str, &str, &str, &str, u64)] = &[
    ("builtin:workspace_run_command", "Run", "execute", "model_visible", 0xaf509ca0aed2dd54),
    ("builtin:ask_user", "Ask", "interactive", "model_visible", 0x3151a4babe493151),
    ("builtin:workspace_gather_context", "Read", "read", "model_visible", 0x5af475abe80c4c20),
    ("builtin:workspace_read_document", "ReadDocument", "read", "model_visible", 0x5769b65e6b8a7858),
    ("builtin:workspace_list_files", "List", "read", "model_visible", 0x01f988b03f5936aa),
    ("builtin:workspace_search_text", "Search", "search", "model_visible", 0xedde05539ffda305),
    ("builtin:workspace_glob_files", "Glob", "read", "model_visible", 0x7d7b2f45daf2ad24),
    ("builtin:web_fetch_url", "WebFetch", "read", "model_visible", 0x53548f927d9049d8),
    ("builtin:web_search_query", "WebSearch", "search", "model_visible", 0xe28717c7e81f74c2),
    ("builtin:mcp_resource_read", "MCPResource", "read", "model_visible", 0x06d727013908acfc),
    ("builtin:tool_search", "ToolSearch", "search", "deferred", 0xccee49ae3acd7fa1),
    ("builtin:workspace_write_file", "Write", "write", "model_visible", 0x5196db9d3c2fc7be),
    ("builtin:workspace_edit_file", "Edit", "write", "model_visible", 0x5247515e4c7ae95b),
    ("builtin:workspace_batch", "BatchExecute", "composite", "model_visible", 0x8c3195523121c376),
    // `abort` op 由 b6fd928d（[decl-change-approved]）加入模型可见输入面，schema 逐字节变化属**已批准**变更，
    // golden 随之同步；日后再变必须重新走 decl-change 评审，不得照抄实测值。
    ("builtin:plan_control", "Plan", "write", "model_visible", 0x1e6b65963cc9549a),
    ("builtin:view_image", "ViewImage", "read", "model_visible", 0x2e5e324a17a5cab2),
    ("builtin:subagent", "subagent", "composite", "model_visible", 0xc9f3f6ae8f81c76a),
    ("builtin:spawn_teammate", "spawn_teammate", "interactive", "model_visible", 0xf884da9528ced99b),
    ("builtin:list_agents", "list_agents", "interactive", "model_visible", 0x439ad3b9da88d665),
    ("builtin:send_message", "send_message", "interactive", "model_visible", 0xd06e563247ae205e),
    ("builtin:interrupt_agent", "interrupt_agent", "interactive", "model_visible", 0x0495e2c07dcf55e4),
    ("builtin:drain_inbox", "drain_inbox", "interactive", "model_visible", 0xff4f114de2906fae),
    ("builtin:team_task_create", "team_task_create", "write", "model_visible", 0x36fa58e2c8eb4a39),
    ("builtin:team_task_update", "team_task_update", "write", "model_visible", 0x26065cf658e7069b),
    ("builtin:team_task_list", "team_task_list", "read", "model_visible", 0x439ad3b9da88d665),
    ("builtin:todo_write", "TodoWrite", "write", "model_visible", 0xe360c203e8aade25),
    ("builtin:todo_list", "TodoList", "read", "model_visible", 0x1cfdceeb1510d242),
    ("builtin:spill_read", "SpillRead", "read", "deferred", 0xa4189c78e60e6915),
    ("builtin:create_goal", "create_goal", "external", "model_visible", 0x1fcc5420678b51d5),
    ("builtin:get_goal", "get_goal", "external", "model_visible", 0x1cfdceeb1510d242),
    ("builtin:update_goal", "update_goal", "external", "model_visible", 0xbab3de6f3c5e14cd),
    ("builtin:present", "Present", "read", "model_visible", 0xe92daba81a9b350d),
    ("builtin:fuzzy_file_search", "Find", "search", "model_visible", 0x0a29b307e2ebe54d),
    ("builtin:git_diff_remote", "GitDiff", "read", "model_visible", 0x701e2ff535594884),
    ("builtin:job_start", "JobStart", "execute", "model_visible", 0x93d777a699425df8),
    ("builtin:job_output", "JobOutput", "execute", "model_visible", 0x983d0331bc8eea5d),
    ("builtin:job_kill", "JobKill", "execute", "model_visible", 0x48dc1dc492ee2adc),
    ("builtin:job_list", "JobList", "execute", "model_visible", 0x439ad3b9da88d665),
    ("builtin:skill_load", "skill_load", "read", "model_visible", 0xe6650761f1de664f),
    ("builtin:time_now", "Run", "read", "internal", 0x439ad3b9da88d665),
    ("builtin:echo_input", "Ask", "interactive", "internal", 0x71e16d17e1847e86),
    ("builtin:workspace_read_file", "Read", "read", "internal", 0xbdd5da7930910554),
    ("builtin:workspace_read_file_segment", "Read", "read", "internal", 0xaf73ba0df1bb2c5c),
    ("builtin:workspace_path_info", "List", "read", "deferred", 0x62cee6a8c448a494),
    ("builtin:workflow", "workflow", "composite", "internal", 0xaa6e82a1e1a8977b),
];

#[test]
fn t1_builtin_surface_identity_is_frozen() {
    // Arrange
    let registry = builtin_snapshot();

    // Act
    let actual: Vec<(&str, &str, &str, &str, u64)> = registry
        .descriptors
        .iter()
        .map(|descriptor| {
            let schema = serde_json::to_string(&descriptor.input_schema).expect("schema json");
            (
                descriptor.identity.descriptor_id.as_str(),
                descriptor.identity.model_name.as_str(),
                descriptor.kind.as_str(),
                descriptor.exposure.as_str(),
                fnv1a(schema.as_bytes()),
            )
        })
        .collect();

    // Assert 1 — 数量冻结
    assert_eq!(
        registry.descriptors.len(),
        45,
        "内建 descriptor 数量必须保持 45，实际 {}",
        registry.descriptors.len()
    );
    assert_eq!(
        actual.len(),
        BUILTIN_GOLDEN.len(),
        "golden 清单长度必须与实现一致"
    );

    // Assert 2 — 逐条冻结（每条单独给出差异，禁止整批笼统断言）
    for (index, (expected, observed)) in BUILTIN_GOLDEN.iter().zip(actual.iter()).enumerate() {
        assert_eq!(
            observed.0, expected.0,
            "第 {index} 个 descriptor 的 descriptor_id 变化（名称/顺序不变量）"
        );
        assert_eq!(
            observed.1, expected.1,
            "第 {index} 个 descriptor 的 model_name 变化（模型面契约）"
        );
        assert_eq!(observed.2, expected.2, "`{}` 的 kind 变化", expected.0);
        assert_eq!(observed.3, expected.3, "`{}` 的 exposure 变化", expected.0);
        assert_eq!(
            observed.4, expected.4,
            "`{}` 的 input_schema 逐字节变化（KV-cache 前缀 / 模型契约不变量）",
            expected.0
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 2. 45 primitive 的分派覆盖（行为遍历，不断言表顺序）
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// 从 descriptor 的 input schema 生成最小合法入参（只填 `required` 里的键）。
///
/// 目的：让断言停在「分派表有没有这一臂」，而不是停在 schema 校验
/// （`invalid_arguments` 发生在 handler 之前，会造成"覆盖已存在"的假绿）。
fn minimal_arguments(schema: &Value) -> Value {
    let mut object = serde_json::Map::new();
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for key in required {
        let Some(name) = key.as_str() else { continue };
        let property = schema
            .get("properties")
            .and_then(|properties| properties.get(name))
            .cloned()
            .unwrap_or(Value::Null);
        let value = match property.get("type").and_then(Value::as_str) {
            Some("integer") => json!(1),
            Some("number") => json!(1),
            Some("boolean") => json!(false),
            Some("array") => json!([]),
            Some("object") => json!({}),
            _ => json!("probe"),
        };
        object.insert(name.to_string(), value);
    }
    Value::Object(object)
}

#[test]
fn t1_all_45_builtin_primitives_reach_a_handler() {
    // Arrange — 生产装配（唯一一处注册全部内建 handler 的路径）
    let root = TempRoot::new("coverage");
    let executor = build_governed_executor(Some(root.path.clone()), None);
    let dispatcher = executor.dispatcher();
    let descriptors = dispatcher.registry().descriptors.clone();
    let context = DispatchContext::default();

    // Act + Assert 1 — 45 个 primitive 逐个遍历，每个都必须落到 handler
    let mut unreachable: Vec<String> = Vec::new();
    for descriptor in &descriptors {
        let descriptor_id = descriptor.identity.descriptor_id.clone();
        let outcome = dispatcher.dispatch_governed(
            request(
                InvocationOrigin::System,
                &descriptor_id,
                &format!("coverage-{descriptor_id}"),
                minimal_arguments(&descriptor.input_schema),
            ),
            &context,
        );
        let code = outcome_error_code(&outcome);
        for forbidden in ["unsupported_tool", "no_handler_registered"] {
            if code.as_deref() == Some(forbidden) {
                unreachable.push(format!("{descriptor_id} -> {forbidden}"));
            }
        }
    }

    // Assert 2 — 失败清单逐条可读（不是"某个工具失败了"这种笼统信息）
    assert!(
        unreachable.is_empty(),
        "以下内建 primitive 没有分派路径（分派表覆盖不完整）：{unreachable:#?}"
    );
    assert_eq!(
        descriptors.len(),
        45,
        "遍历基数必须恰好是 45 个内建 primitive，实际 {}",
        descriptors.len()
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 3. `unsupported_tool` 只表示「名字确实未知」
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn t1_unknown_tool_name_is_unsupported() {
    // Arrange
    let root = TempRoot::new("unknown");
    let router = ToolRouter::with_workspace_root(root.path.clone());

    // Act
    let result = router.execute(&ToolCall {
        call_id: None,
        name: "definitely_not_a_tool_xyz".to_string(),
        arguments: json!({}),
        plan: None,
    });

    // Assert
    let code = serde_json::from_str::<Value>(&result.output)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    assert_eq!(
        code.as_deref(),
        Some("unsupported_tool"),
        "未知工具名必须报 `unsupported_tool`，实际 {code:?}（输出：{}）",
        result.output
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 4. 未注册 handler 的 descriptor 必须 fail-closed
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn t1_descriptor_without_handler_fails_closed() {
    // Arrange — descriptor 在 registry 里（模型可见），但没有任何 handler / provider
    let descriptor = external_descriptor(
        "demo",
        "orphan_tool",
        ToolKind::Read,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let descriptor_id = descriptor.identity.descriptor_id.clone();
    let registry = Arc::new(registry_with("orphan-surface", vec![descriptor]));
    let dispatcher = GovernedDispatcher::new(Arc::clone(&registry), Arc::new(FakeClock::new(0)));
    dispatcher.set_turn_view(registry.default_turn_tool_view());

    // Act
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &descriptor_id,
            "orphan-call",
            json!({}),
        ),
        &DispatchContext::default(),
    );

    // Assert — fail-closed，且错误码区分「有 descriptor 但没执行体」
    assert_eq!(
        outcome.execution_status,
        ToolExecutionStatus::Error,
        "未注册 handler 的 descriptor 必须 fail-closed，实际输出：{}",
        outcome_output(&outcome)
    );
    let code = outcome_error_code(&outcome);
    assert_eq!(
        code.as_deref(),
        Some("no_handler_registered"),
        "缺少执行体必须以 `no_handler_registered` 拒绝（不得伪装成 unsupported_tool），实际 {code:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 5. 只读模式：按 descriptor 声明的权限事实判定，不按名字嗅探
// ─────────────────────────────────────────────────────────────────────────────────────────────

fn read_only_context() -> DispatchContext {
    DispatchContext {
        tool_authorization_mode: Some("read_only".to_string()),
        ..Default::default()
    }
}

fn read_only_dispatcher(
    descriptor: ToolDescriptor,
    handler_invocations: Arc<AtomicUsize>,
) -> GovernedDispatcher {
    let descriptor_id = descriptor.identity.descriptor_id.clone();
    let registry = Arc::new(registry_with("read-only-surface", vec![descriptor]));
    let dispatcher = GovernedDispatcher::new(Arc::clone(&registry), Arc::new(FakeClock::new(0)));
    // 生产装配用的策略评估器（read_only 闸门在这里；dispatcher 默认评估器没有该分支）
    dispatcher.register_policy_evaluator(Arc::new(GovernedPolicyEvaluator::default()));
    dispatcher.register_handler(
        descriptor_id,
        Arc::new(CountingHandler(handler_invocations)) as Arc<dyn PrimitiveToolHandler>,
    );
    dispatcher.set_turn_view(registry.default_turn_tool_view());
    dispatcher
}

#[test]
fn t1_read_only_denies_external_write_tool_by_declared_permission() {
    // Arrange — 外部写工具：声明 workspace.write，但 kind=External 且名字里没有 write/edit/bash
    let descriptor = external_descriptor(
        "demo",
        "persist_blob",
        ToolKind::External,
        &[ToolPermissionScope::WorkspaceWrite],
    );
    let descriptor_id = descriptor.identity.descriptor_id.clone();
    let invocations = Arc::new(AtomicUsize::new(0));
    let dispatcher = read_only_dispatcher(descriptor, Arc::clone(&invocations));

    // Act
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &descriptor_id,
            "read-only-external-write",
            json!({ "payload": "should never be written" }),
        ),
        &read_only_context(),
    );

    // Assert 1 — 必须被拒（按名嗅探的旧实现会放行 → 这正是本用例的红相来源）
    assert_eq!(
        outcome.execution_status,
        ToolExecutionStatus::Error,
        "只读模式下，声明了 workspace.write 的外部工具必须被拒绝（按名嗅探的实现会放行）。实际输出：{}",
        outcome_output(&outcome)
    );
    let code = outcome_error_code(&outcome);
    assert_eq!(
        code.as_deref(),
        Some("permission_denied"),
        "只读模式拒绝必须以 `permission_denied` 表达，实际 {code:?}"
    );

    // Assert 2 — 必须是 Deny 而不是 Ask（只读模式不给"审批后仍可写"的余地）
    assert!(
        outcome.control_outcome.is_none(),
        "只读模式下的外部写工具必须是 Deny，不得降级成审批挂起（Ask）"
    );

    // Assert 3 — handler 根本不得被执行（拒绝必须发生在执行之前）
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        0,
        "只读模式拒绝后不得执行 handler（否则写副作用已经发生）"
    );
}

#[test]
fn t1_read_only_allows_external_read_tool() {
    // Arrange — 同形状的外部工具，但只声明 workspace.read
    let descriptor = external_descriptor(
        "demo",
        "inspect_blob",
        ToolKind::External,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let descriptor_id = descriptor.identity.descriptor_id.clone();
    let invocations = Arc::new(AtomicUsize::new(0));
    let dispatcher = read_only_dispatcher(descriptor, Arc::clone(&invocations));

    // Act
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &descriptor_id,
            "read-only-external-read",
            json!({ "payload": "read me" }),
        ),
        &read_only_context(),
    );

    // Assert — 只读工具必须照常执行（拒绝逻辑不得退化成"外部工具一律拒绝"）
    assert_eq!(
        outcome.execution_status,
        ToolExecutionStatus::Ok,
        "只读模式下声明 workspace.read 的外部工具必须可执行，实际输出：{}",
        outcome_output(&outcome)
    );
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "只读工具必须真正执行一次 handler"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 6. 外部 ToolProvider 经 GovernedDispatcher 全链路（授权 / 预算 / 生命周期 / 结果）
// ─────────────────────────────────────────────────────────────────────────────────────────────

fn dispatcher_with_provider(
    provider: Arc<RecordingProvider>,
) -> (GovernedDispatcher, Arc<RecordingObserver>) {
    let dispatcher = GovernedDispatcher::new(
        Arc::new(builtin_snapshot()),
        Arc::new(FakeClock::new(0)),
    );
    let observer = Arc::new(RecordingObserver::default());
    dispatcher.register_lifecycle_observer(
        Arc::clone(&observer) as Arc<dyn DispatchLifecycleObserver>,
    );
    dispatcher
        .register_tool_provider(Arc::clone(&provider) as Arc<dyn ToolProvider>)
        .unwrap_or_else(|err| panic!("注册外部 ToolProvider 必须成功，实际错误：{err}"));
    // 会话内冻结：新接入来源在下一轮生效（此处显式重建 turn view 模拟下一轮）
    dispatcher.set_turn_view(dispatcher.registry().default_turn_tool_view());
    (dispatcher, observer)
}

#[test]
fn t1_external_tool_dispatches_through_governed_pipeline() {
    // Arrange
    let descriptor = external_descriptor(
        "demo",
        "echo_tool",
        ToolKind::Read,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let descriptor_id = descriptor.identity.descriptor_id.clone();
    let provider = Arc::new(RecordingProvider::new(
        "demo",
        vec![descriptor],
        ProviderBehavior::Return("external-payload-42".to_string()),
    ));
    let (dispatcher, observer) = dispatcher_with_provider(Arc::clone(&provider));

    // Assert 0 — descriptor 已进入 registry 且对模型可见（单一真值源）
    let visible = dispatcher
        .registry()
        .resolve("mcp__demo__echo_tool")
        .map(|descriptor| descriptor.identity.descriptor_id.clone());
    assert_eq!(
        visible.as_deref(),
        Some(descriptor_id.as_str()),
        "provider 的 descriptor 必须进入 registry 并按 model_name 可解析"
    );
    assert!(
        dispatcher.turn_view().allows_model_descriptor(&descriptor_id),
        "外部 descriptor 必须进入 turn view（否则模型永远看不到外部工具）"
    );

    // Act
    let arguments = json!({ "text": "hello-external", "n": 3 });
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &descriptor_id,
            "external-call-1",
            arguments.clone(),
        ),
        &DispatchContext::default(),
    );

    // Assert 1 — 结果来自 provider，且经治理管线返回
    assert_eq!(
        outcome.execution_status,
        ToolExecutionStatus::Ok,
        "外部工具必须经 GovernedDispatcher 成功返回，实际输出：{}",
        outcome_output(&outcome)
    );
    let output = outcome_output(&outcome);
    assert!(
        output.contains("external-payload-42"),
        "调用结果必须来自 provider 返回值，实际输出：{output}"
    );

    // Assert 2 — provider 收到的是稳定 descriptor_id 与本次参数
    let calls = provider.calls();
    assert_eq!(
        calls.len(),
        1,
        "一次 dispatch 必须恰好调用 provider 一次，实际 {}",
        calls.len()
    );
    assert_eq!(
        calls[0].0, descriptor_id,
        "provider 必须收到 descriptor_id（不是 model_name / 别名）"
    );
    assert_eq!(calls[0].1, arguments, "provider 必须收到本次调用的参数");

    // Assert 3 — 授权步骤真实发生（不是旁路直达 handler）
    let records = observer.records();
    assert_eq!(records.len(), 1, "每次 top-level 调用必须恰好一条生命周期记录");
    assert_eq!(records[0].descriptor_id, descriptor_id);
    assert_eq!(
        records[0].phase,
        DispatchLifecyclePhase::Completed,
        "成功调用必须记 Completed 相位"
    );
    assert_eq!(
        records[0].decision,
        Some(PermissionVerdict::Allow),
        "外部工具必须走完授权步骤并留下 Allow 证据"
    );
}

#[test]
fn t1_external_provider_error_code_keeps_its_source() {
    // Arrange — provider 以分源错误码失败（ADR 0065 决策 9）
    let descriptor = external_descriptor(
        "demo",
        "slow_tool",
        ToolKind::Read,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let descriptor_id = descriptor.identity.descriptor_id.clone();
    let provider = Arc::new(RecordingProvider::new(
        "demo",
        vec![descriptor],
        ProviderBehavior::Fail(ToolProviderError::new(
            "mcp_timeout",
            "fixture provider timed out",
        )),
    ));
    let (dispatcher, _observer) = dispatcher_with_provider(provider);

    // Act
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &descriptor_id,
            "external-call-timeout",
            json!({}),
        ),
        &DispatchContext::default(),
    );

    // Assert — 不得塌缩成 handler_error，错误码必须保留来源
    let code = outcome_error_code(&outcome);
    assert_eq!(
        code.as_deref(),
        Some("mcp_timeout"),
        "外部来源错误码必须原样透出（不得塌缩成 handler_error），实际 {code:?}，输出：{}",
        outcome_output(&outcome)
    );
}

#[test]
fn t1_external_provider_output_budget_is_enforced() {
    // Arrange
    let descriptor = external_descriptor(
        "demo",
        "huge_tool",
        ToolKind::Read,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let descriptor_id = descriptor.identity.descriptor_id.clone();
    let provider = Arc::new(RecordingProvider::new(
        "demo",
        vec![descriptor],
        ProviderBehavior::Return("x".repeat(4_096)),
    ));
    let (dispatcher, _observer) = dispatcher_with_provider(provider);
    dispatcher.set_budget_config(DispatchBudgetConfig {
        max_result_bytes: 16,
        unbounded_output_and_deadline: false,
        ..Default::default()
    });

    // Act
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &descriptor_id,
            "external-call-budget",
            json!({}),
        ),
        &DispatchContext::default(),
    );

    // Assert — 预算闸门必须对**外部**工具同样生效
    let code = outcome_error_code(&outcome);
    assert_eq!(
        code.as_deref(),
        Some("output_budget_exceeded"),
        "外部工具的输出必须受预算闸门约束，实际 {code:?}，输出：{}",
        outcome_output(&outcome)
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 7. swap_tool_surface 原子换入
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// 为一个**自洽**的面构造 handler 表：可见 descriptor 全都有执行体。
///
/// 原子换入的语义是「快照 + handler 表」整体生效，因此调用方必须能给出一个自洽的面；
/// 内建 descriptor 用回声 handler 占位即可（本用例只派发外部工具，不验证内建行为）。
fn coherent_handlers(registry: &ToolRegistrySnapshot) -> Vec<(String, Arc<dyn PrimitiveToolHandler>)> {
    registry
        .descriptors
        .iter()
        .map(|descriptor| {
            (
                descriptor.identity.descriptor_id.clone(),
                Arc::new(EchoHandler) as Arc<dyn PrimitiveToolHandler>,
            )
        })
        .collect()
}

#[test]
fn t1_swap_tool_surface_exposes_and_executes_together() {
    // Arrange — 初始面：内建 + alpha；目标面：内建 + beta（alpha 被换出）
    let alpha = external_descriptor(
        "demo",
        "alpha_tool",
        ToolKind::Read,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let initial = Arc::new(registry_with("surface-initial", vec![alpha]));
    let dispatcher = GovernedDispatcher::new(Arc::clone(&initial), Arc::new(FakeClock::new(0)));

    let beta = external_descriptor(
        "demo",
        "beta_tool",
        ToolKind::Read,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let beta_id = beta.identity.descriptor_id.clone();
    let next_surface = registry_with("surface-beta", vec![beta.clone()]);
    let handlers = coherent_handlers(&next_surface);

    // Act
    dispatcher
        .swap_tool_surface("surface-beta", next_surface, handlers)
        .expect("合法快照 + 覆盖其 descriptor 的 handler 表必须换入成功");

    // Assert 1 — 快照身份已切换（单一真值源随之切换）
    assert_eq!(
        dispatcher.registry().snapshot_id,
        "surface-beta",
        "换入后 registry 快照 id 必须等于本次换入的快照"
    );

    // Assert 2 — 可见性与可执行性同时成立（不存在"可见但不可执行"的窗口）
    assert!(
        dispatcher.registry().resolve("mcp__demo__beta_tool").is_some(),
        "换入后 descriptor 必须立即可见"
    );
    assert!(
        dispatcher.registry().resolve("mcp__demo__alpha_tool").is_none(),
        "换入后旧面的 descriptor 必须同时不可见（原子换入，不是无界合并）"
    );

    dispatcher.set_turn_view(dispatcher.registry().default_turn_tool_view());
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &beta_id,
            "swap-exec-1",
            json!({ "k": "v" }),
        ),
        &DispatchContext::default(),
    );
    assert_eq!(
        outcome.execution_status,
        ToolExecutionStatus::Ok,
        "换入后必须立即可执行（可见即可执行），实际输出：{}",
        outcome_output(&outcome)
    );
    assert_eq!(
        outcome_error_code(&outcome),
        None,
        "换入后的首次调用不得出现任何错误码"
    );
}

#[test]
fn t1_swap_never_leaves_a_visible_but_unexecutable_descriptor() {
    // Arrange — 新快照含 descriptor `gamma`，但 handler 表为空
    let gamma = external_descriptor(
        "demo",
        "gamma_tool",
        ToolKind::Read,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let gamma_id = gamma.identity.descriptor_id.clone();
    let initial = Arc::new(builtin_snapshot());
    let dispatcher = GovernedDispatcher::new(Arc::clone(&initial), Arc::new(FakeClock::new(0)));
    let before = dispatcher.registry().snapshot_id.clone();
    let next_surface = registry_with("surface-gamma", vec![gamma]);

    // Act — 换入一个"可见但没有执行体"的面
    let swapped = dispatcher.swap_tool_surface("surface-gamma", next_surface, Vec::new());

    // Assert — ADR 0065 决策 3 的不变量：任一时刻不存在「descriptor 已可见但 handler 未就绪」
    if swapped.is_err() {
        // 首选语义：直接拒绝半换入，且旧面保持不变
        assert_eq!(
            dispatcher.registry().snapshot_id,
            before,
            "换入被拒绝后旧快照必须保持不变（不得留下半换入状态）"
        );
        assert!(
            dispatcher.registry().resolve("mcp__demo__gamma_tool").is_none(),
            "换入被拒绝后新 descriptor 不得意外可见"
        );
        return;
    }

    // 备选语义：允许换入，但那条 descriptor 不得"可见却不可执行"
    if dispatcher.registry().resolve("mcp__demo__gamma_tool").is_some() {
        dispatcher.set_turn_view(dispatcher.registry().default_turn_tool_view());
        let outcome = dispatcher.dispatch_governed(
            request(
                InvocationOrigin::Model,
                &gamma_id,
                "swap-gamma-exec",
                json!({}),
            ),
            &DispatchContext::default(),
        );
        assert_ne!(
            outcome_error_code(&outcome).as_deref(),
            Some("no_handler_registered"),
            "存在「descriptor 已可见但 handler 未就绪」的窗口（违反 ADR 0065 决策 3 的原子性）"
        );
    }
}

#[test]
fn t1_swap_with_inconsistent_handler_table_keeps_the_atomicity_invariant() {
    // Arrange — handler 表与快照不一致：handler 指向快照里不存在的 descriptor
    let initial = Arc::new(builtin_snapshot());
    let dispatcher = GovernedDispatcher::new(Arc::clone(&initial), Arc::new(FakeClock::new(0)));
    let before = dispatcher.registry().snapshot_id.clone();

    let delta = external_descriptor(
        "demo",
        "delta_tool",
        ToolKind::Read,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let delta_id = delta.identity.descriptor_id.clone();
    let next_surface = registry_with("surface-delta", vec![delta]);
    let ghost_handlers: Vec<(String, Arc<dyn PrimitiveToolHandler>)> = vec![(
        "mcp:demo:ghost_tool".to_string(),
        Arc::new(EchoHandler) as Arc<dyn PrimitiveToolHandler>,
    )];

    // Act
    let swapped = dispatcher.swap_tool_surface("surface-delta", next_surface, ghost_handlers);

    // Assert — Lead 裁决 b：不强制必须 Err，但「要么整体拒绝并保留旧面，要么换入后
    // 不存在可见却不可执行」这一原子性不变量必须成立。
    if swapped.is_err() {
        assert_eq!(
            dispatcher.registry().snapshot_id,
            before,
            "换入被拒绝后旧快照必须原样保留（原子性：不得半换入）"
        );
        assert!(
            dispatcher
                .registry()
                .resolve("mcp__demo__delta_tool")
                .is_none(),
            "换入被拒绝后新 descriptor 不得可见"
        );
        return;
    }

    if dispatcher
        .registry()
        .resolve("mcp__demo__delta_tool")
        .is_some()
    {
        dispatcher.set_turn_view(dispatcher.registry().default_turn_tool_view());
        let outcome = dispatcher.dispatch_governed(
            request(InvocationOrigin::Model, &delta_id, "swap-delta-exec", json!({})),
            &DispatchContext::default(),
        );
        assert_ne!(
            outcome_error_code(&outcome).as_deref(),
            Some("no_handler_registered"),
            "新快照可见的 descriptor 没有对应 handler（原子换入被破坏）"
        );
    }
}

#[test]
fn t1_swap_is_atomic_under_concurrent_dispatch() {
    // Arrange — 两个都含 `shared_tool` 的面；并发派发 + 反复换入
    let shared = external_descriptor(
        "demo",
        "shared_tool",
        ToolKind::Read,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let shared_id = shared.identity.descriptor_id.clone();
    let surface_a = registry_with("surface-a", vec![shared.clone()]);

    let dispatcher = GovernedDispatcher::new(
        Arc::new(builtin_snapshot()),
        Arc::new(FakeClock::new(0)),
    );
    let initial_handlers = coherent_handlers(&surface_a);
    dispatcher
        .swap_tool_surface("surface-a", surface_a, initial_handlers)
        .expect("首次换入必须成功");
    dispatcher.set_turn_view(dispatcher.registry().default_turn_tool_view());

    let violations = Arc::new(Mutex::new(Vec::<String>::new()));
    let stop = Arc::new(AtomicUsize::new(0));

    // Act — 4 个并发派发线程
    let mut workers = Vec::new();
    for worker in 0..4 {
        let dispatcher = dispatcher.clone();
        let descriptor_id = shared_id.clone();
        let violations = Arc::clone(&violations);
        let stop = Arc::clone(&stop);
        workers.push(std::thread::spawn(move || {
            let mut index = 0usize;
            while stop.load(Ordering::SeqCst) == 0 && index < 40 {
                let outcome = dispatcher.dispatch_governed(
                    request(
                        InvocationOrigin::System,
                        &descriptor_id,
                        &format!("race-{worker}-{index}"),
                        json!({ "worker": worker, "index": index }),
                    ),
                    &DispatchContext::default(),
                );
                if let Some(code) = outcome_error_code(&outcome) {
                    violations
                        .lock()
                        .expect("violation mutex poisoned")
                        .push(format!("worker {worker} iteration {index}: {code}"));
                }
                index += 1;
            }
        }));
    }

    // 主线程交替换入 a / b
    for round in 0..60 {
        let (snapshot_id, surface) = if round % 2 == 0 {
            (
                "surface-b",
                registry_with("surface-b", vec![shared_for_race(&shared_id)]),
            )
        } else {
            (
                "surface-a",
                registry_with("surface-a", vec![shared_for_race(&shared_id)]),
            )
        };
        let handlers = coherent_handlers(&surface);
        dispatcher
            .swap_tool_surface(snapshot_id, surface, handlers)
            .unwrap_or_else(|err| panic!("第 {round} 轮换入必须成功，实际错误：{err}"));
    }
    stop.store(1, Ordering::SeqCst);

    for worker in workers {
        worker.join().expect("并发派发线程不得 panic");
    }

    // Assert — 任一时刻都不得观测到"可见但不可执行"的中间态
    let violations = violations.lock().expect("violation mutex poisoned").clone();
    assert!(
        violations.is_empty(),
        "并发换入期间观测到不可执行的调用（存在中间态）：{violations:#?}"
    );
}

/// 并发换入用例每轮都要重建 descriptor（registry 会取得所有权）。
fn shared_for_race(descriptor_id: &str) -> ToolDescriptor {
    let tool = descriptor_id
        .rsplit(':')
        .next()
        .expect("descriptor id must be namespaced");
    external_descriptor(
        "demo",
        tool,
        ToolKind::Read,
        &[ToolPermissionScope::WorkspaceRead],
    )
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 8. 跨通道一致性（Lead 裁决 a：MCP 侧合成 provider 与插件侧 provider 收敛为同一
//    `tools::ToolProvider` 通道，不得各自旁路治理管线）
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// 跨通道 fixture：一个真实进程外插件，声明 `echo`（正常回显）与 `boom`（isError=true）两个工具。
const CROSS_CHANNEL_PLUGIN: &str = r#"#!/usr/bin/env python3
import json
import sys

MARKER = "PLUGIN_CROSS_CHANNEL_MARKER"


def emit(payload):
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()


def respond(request_id, result):
    emit({"jsonrpc": "2.0", "id": request_id, "result": result})


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        message = json.loads(line)
    except json.JSONDecodeError:
        continue
    method = message.get("method")
    request_id = message.get("id")
    params = message.get("params") or {}
    if method == "initialize":
        respond(request_id, {
            "apiVersion": "pony-plugin-v1",
            "capabilities": ["tools"],
            "name": "cross_fixture",
            "version": "0.1.0",
        })
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        respond(request_id, {"tools": [
            {"name": "echo", "description": "cross channel echo", "inputSchema": {"type": "object"}},
            {"name": "boom", "description": "cross channel failure", "inputSchema": {"type": "object"}},
        ]})
    elif method == "tools/call":
        name = params.get("name")
        if name == "boom":
            respond(request_id, {"isError": True, "message": "fixture deliberately failed"})
        else:
            respond(request_id, {"echoed": params.get("arguments"), "marker": MARKER})
    elif method == "shutdown":
        respond(request_id, {})
        break
    else:
        continue
"#;

/// 按模型可见名在 registry 中定位 descriptor。
///
/// 不走 `RegistrySnapshot::resolve`：该 API 只查 alias index，而外部来源是否把 model_name
/// 登记成 alias 属于提供方自由度（本断言只关心「同一 registry 里同时存在两条通道的工具」）。
fn descriptor_by_model_name(
    registry: &ToolRegistrySnapshot,
    model_name: &str,
) -> Option<ToolDescriptor> {
    registry
        .descriptors
        .iter()
        .find(|descriptor| descriptor.identity.model_name == model_name)
        .cloned()
}

fn cross_channel_fixture(root: &TempRoot) -> PathBuf {
    let path = root.path.join("plugin-cross-channel.py");
    std::fs::write(&path, CROSS_CHANNEL_PLUGIN).expect("write cross-channel plugin fixture");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod cross-channel plugin fixture");
    }
    path
}

fn cross_channel_manifest() -> PluginManifest {
    PluginManifest::parse(
        r#"{
          "apiVersion": "pony-plugin-v1",
          "name": "cross_fixture",
          "version": "0.1.0",
          "capabilities": ["tools"],
          "tools": [
            { "name": "echo", "description": "cross channel echo", "inputSchema": { "type": "object" } },
            { "name": "boom", "description": "cross channel failure", "inputSchema": { "type": "object" } }
          ]
        }"#,
    )
    .expect("cross-channel manifest must parse")
}

/// 构造一个同时挂了「合成外部 provider」与「进程外插件 provider」的 dispatcher。
fn cross_channel_dispatcher(
    root: &TempRoot,
    external_behavior: ProviderBehavior,
) -> (GovernedDispatcher, Arc<RecordingProvider>, Arc<RecordingObserver>) {
    // Arrange — 插件侧：真实进程 + 真实握手，再适配成 tools::ToolProvider
    let command = cross_channel_fixture(root);
    let supervisor = PluginSupervisor::spawn(
        cross_channel_manifest(),
        command,
        PluginHostConfig {
            workspace_root: root.path.clone(),
            timeout_ms: 5_000,
        },
    )
    .expect("cross-channel plugin must spawn");
    let plugin_provider = PluginHostProvider::new(supervisor);
    plugin_provider
        .initialize()
        .expect("cross-channel plugin must initialize");

    // Arrange — MCP 侧：合成 provider（同一 trait 的第二个实现）
    let external = Arc::new(RecordingProvider::new(
        "demo",
        vec![external_descriptor(
            "demo",
            "echo_tool",
            ToolKind::Read,
            &[ToolPermissionScope::WorkspaceRead],
        )],
        external_behavior,
    ));

    // Arrange — 同一个 dispatcher：生产策略评估器 + 生命周期观察者
    let dispatcher = GovernedDispatcher::new(
        Arc::new(builtin_snapshot()),
        Arc::new(FakeClock::new(0)),
    );
    dispatcher.register_policy_evaluator(Arc::new(AllowAllEvaluator));
    let observer = Arc::new(RecordingObserver::default());
    dispatcher.register_lifecycle_observer(
        Arc::clone(&observer) as Arc<dyn DispatchLifecycleObserver>,
    );
    dispatcher
        .register_tool_provider(Arc::clone(&external) as Arc<dyn ToolProvider>)
        .unwrap_or_else(|err| panic!("合成外部 provider 注册必须成功，实际错误：{err}"));
    dispatcher
        .register_tool_provider(Arc::new(plugin_provider) as Arc<dyn ToolProvider>)
        .unwrap_or_else(|err| panic!("插件 provider 注册必须成功，实际错误：{err}"));
    dispatcher.set_turn_view(dispatcher.registry().default_turn_tool_view());
    (dispatcher, external, observer)
}

#[test]
fn t1_external_and_plugin_sources_share_one_tool_provider_channel() {
    // Arrange
    let root = TempRoot::new("cross-channel-ok");
    let (dispatcher, external, observer) = cross_channel_dispatcher(
        &root,
        ProviderBehavior::Return("external-payload-777".to_string()),
    );

    // Assert 1 — 两个来源都进了**同一个** registry，且来源身份可区分
    let external_descriptor = descriptor_by_model_name(dispatcher.registry().as_ref(), "mcp__demo__echo_tool")
        .expect("合成外部 descriptor 必须在 registry 中");
    let plugin_descriptor = descriptor_by_model_name(dispatcher.registry().as_ref(), "plugin__cross_fixture__echo")
        .expect("插件 descriptor 必须在 registry 中（命名空间 plugin__<name>__<tool>）");
    assert_eq!(
        external_descriptor.handler_provenance.source_id, "demo",
        "合成 provider 的来源身份必须落在 handler provenance 上"
    );
    assert_eq!(
        plugin_descriptor.handler_provenance.source_id, "plugin:cross_fixture",
        "插件 provider 的来源身份必须落在 handler provenance 上"
    );
    assert_eq!(
        plugin_descriptor.kind,
        ToolKind::External,
        "插件工具必须作为 External kind 进入同一工具面"
    );

    // Act 1 — 同一个 dispatch 入口，两条通道各调一次
    let external_outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &external_descriptor.identity.descriptor_id,
            "cross-external-ok",
            json!({ "text": "external" }),
        ),
        &DispatchContext::default(),
    );
    let plugin_id = plugin_descriptor.identity.descriptor_id.clone();
    let plugin_outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &plugin_id,
            "cross-plugin-ok",
            json!({ "text": "plugin" }),
        ),
        &DispatchContext::default(),
    );

    // Assert 2 — 两条通道都经治理管线成功返回各自的 payload（不是旁路直达）
    assert_eq!(
        external_outcome.execution_status,
        ToolExecutionStatus::Ok,
        "合成外部工具必须成功返回，实际输出：{}",
        outcome_output(&external_outcome)
    );
    assert!(
        outcome_output(&external_outcome).contains("external-payload-777"),
        "合成外部工具的结果必须来自其 provider，实际：{}",
        outcome_output(&external_outcome)
    );
    assert_eq!(
        plugin_outcome.execution_status,
        ToolExecutionStatus::Ok,
        "插件工具必须经同一治理管线成功返回，实际输出：{}",
        outcome_output(&plugin_outcome)
    );
    assert!(
        outcome_output(&plugin_outcome).contains("PLUGIN_CROSS_CHANNEL_MARKER"),
        "插件工具的结果必须来自真实插件进程，实际：{}",
        outcome_output(&plugin_outcome)
    );

    // Assert 3 — 生命周期证据同形：两条通道各一条 Completed + Allow
    let records = observer.records();
    assert_eq!(
        records.len(),
        2,
        "两条通道各必须恰好一条生命周期记录，实际 {records:?}"
    );
    for record in &records {
        assert_eq!(
            record.phase,
            DispatchLifecyclePhase::Completed,
            "`{}` 必须记 Completed", record.descriptor_id
        );
        assert_eq!(
            record.decision,
            Some(PermissionVerdict::Allow),
            "`{}` 必须走完授权步骤并留下 Allow 证据", record.descriptor_id
        );
    }
    let observed_ids: BTreeSet<String> = records
        .iter()
        .map(|record| record.descriptor_id.clone())
        .collect();
    assert_eq!(
        observed_ids,
        BTreeSet::from([
            external_descriptor.identity.descriptor_id.clone(),
            plugin_id.clone(),
        ]),
        "生命周期记录必须分别对应两条通道的 descriptor_id"
    );

    // Assert 4 — 合成 provider 收到的是 descriptor_id（不是 model_name）：同一入口的判据
    let calls = external.calls();
    assert_eq!(calls.len(), 1, "合成 provider 必须被调用一次");
    assert_eq!(
        calls[0].0, external_descriptor.identity.descriptor_id,
        "两条通道都必须以 descriptor_id 作为分派真值"
    );
}

#[test]
fn t1_external_and_plugin_failures_keep_their_own_source_families() {
    // Arrange — 合成 provider 以 `mcp_*` 失败；插件以 `plugin_*` 失败
    let root = TempRoot::new("cross-channel-fail");
    let (dispatcher, _external, _observer) = cross_channel_dispatcher(
        &root,
        ProviderBehavior::Fail(ToolProviderError::new("mcp_timeout", "fixture mcp timeout")),
    );
    let external_id = descriptor_by_model_name(dispatcher.registry().as_ref(), "mcp__demo__echo_tool")
        .expect("合成外部 descriptor 必须在 registry 中")
        .identity
        .descriptor_id;
    let plugin_boom_id = descriptor_by_model_name(dispatcher.registry().as_ref(), "plugin__cross_fixture__boom")
        .expect("插件失败工具必须在 registry 中")
        .identity
        .descriptor_id;

    // Act
    let external_outcome = dispatcher.dispatch_governed(
        request(InvocationOrigin::Model, &external_id, "cross-external-fail", json!({})),
        &DispatchContext::default(),
    );
    let plugin_outcome = dispatcher.dispatch_governed(
        request(InvocationOrigin::Model, &plugin_boom_id, "cross-plugin-fail", json!({})),
        &DispatchContext::default(),
    );

    // Assert 1 — 合成外部通道保留 `mcp_*` 家族
    let external_code = outcome_error_code(&external_outcome);
    assert_eq!(
        external_code.as_deref(),
        Some("mcp_timeout"),
        "合成外部通道必须保留 mcp_* 来源码，实际 {external_code:?}，输出：{}",
        outcome_output(&external_outcome)
    );

    // Assert 2 — 插件通道保留 `plugin_*` 家族
    let plugin_code = outcome_error_code(&plugin_outcome);
    let plugin_code = plugin_code.expect("插件失败必须带结构化错误码");
    assert!(
        plugin_code.starts_with("plugin_"),
        "插件通道必须保留 plugin_* 来源码，实际 `{plugin_code}`，输出：{}",
        outcome_output(&plugin_outcome)
    );

    // Assert 3 — 两条通道都不得塌缩成通用 handler_error（跨通道一致性的核心）
    for (label, code) in [("external", &external_code), ("plugin", &Some(plugin_code.clone()))] {
        assert_ne!(
            code.as_deref(),
            Some("handler_error"),
            "{label} 通道的错误码塌缩成 handler_error（来源信息丢失）"
        );
        assert_ne!(
            code.as_deref(),
            Some("unsupported_tool"),
            "{label} 通道误用 `unsupported_tool`（该码只表示名字确实未知）"
        );
    }

    // Assert 4 — 两个家族必须可区分（不是同一个笼统码）
    assert_ne!(
        plugin_code, "mcp_timeout",
        "插件失败不得冒用 MCP 来源码"
    );
}


// ─────────────────────────────────────────────────────────────────────────────────────────────
// 9. alias 口径（ADR 0065 决策 7）：外部 descriptor 的 model_name 必须可按名解析
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn t1_external_model_names_resolve_through_the_registry_alias_index() {
    // Arrange — 同一 dispatcher 上两条外部通道（合成 MCP 侧 + 真实插件侧）
    let root = TempRoot::new("alias");
    let (dispatcher, _external, _observer) = cross_channel_dispatcher(
        &root,
        ProviderBehavior::Return("alias-check".to_string()),
    );

    // Act + Assert — 两侧模型名都必须能按 alias 命中，且口径一致
    let registry = dispatcher.registry();
    for model_name in ["mcp__demo__echo_tool", "plugin__cross_fixture__echo"] {
        let resolved = registry
            .resolve(model_name)
            .unwrap_or_else(|| {
                panic!("`resolve(\"{model_name}\")` 必须命中（ADR 0065 决策 7：外部 descriptor 必须把 model_name 登记为 alias）")
            });
        assert_eq!(
            resolved.identity.model_name, model_name,
            "按名解析必须回落到同名 descriptor（不得按前缀猜）"
        );
        assert!(
            resolved.aliases.iter().any(|alias| alias == model_name),
            "`{model_name}` 必须出现在 descriptor 的 aliases 里，实际 {:?}",
            resolved.aliases
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 10. [KV-CACHE-PREFIX] 追加 1 个外部工具后，内建 provider 视图必须逐字节不变
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// 断言的是**模型可见工具面数组 → provider 合同视图**这条投影：
/// 外部工具只允许稳定追加在内建之后，内建前缀一个字节都不许动（否则 KV-cache 前缀崩塌）。
///
/// `[CONTRACT-PENDING]`：本用例依赖一个公开投影入口
/// `provider::surface_contract_views(&[ToolDefinition]) -> Vec<ToolDefinitionContractView>`。
/// 该入口是这段不变量唯一的可观测面（其余路径需要真实 API key 才能触达）。
#[test]
fn t1_appending_one_external_tool_keeps_the_builtin_provider_view_byte_identical() {
    // Arrange — 内建工具面数组（45 条定义，其中 38 条进入模型可见投影）
    let baseline_tools = builtin_tools();
    let mut with_external = baseline_tools.clone();
    let external_name: &'static str = Box::leak("mcp__x__y".to_string().into_boxed_str());
    with_external.push(ToolDefinition {
        name: external_name,
        description: "synthetic external tool appended by the acceptance test",
        input_schema: json!({ "type": "object", "properties": {} }),
    });

    // Act
    let baseline_views = surface_contract_views(&baseline_tools);
    let observed_views = surface_contract_views(&with_external);

    // Assert 1 — 内建模型可见面必须是 38 项（数量不因外部追加而漂移）
    assert_eq!(
        baseline_views.len(),
        38,
        "内建模型可见工具面必须恰好 38 项，实际 {}",
        baseline_views.len()
    );
    assert_eq!(
        observed_views.len(),
        39,
        "追加 1 个外部工具后必须恰好 39 项（38 内建 + 1 外部），实际 {}（疑似落入 legacy fallback）",
        observed_views.len()
    );

    // Assert 2 — 内建前缀逐字节不变（逐项比较 + JSON 字节比较）
    for (index, (expected, observed)) in baseline_views.iter().zip(observed_views.iter()).enumerate() {
        assert_eq!(
            observed, expected,
            "追加外部工具后第 {index} 项内建视图发生变化（{} → {}），KV-cache 前缀崩塌",
            expected.name, observed.name
        );
        let expected_json = serde_json::to_string(expected).expect("baseline view json");
        let observed_json = serde_json::to_string(observed).expect("observed view json");
        assert_eq!(
            observed_json, expected_json,
            "第 {index} 项内建视图的序列化字节发生变化（KV-cache 前缀不变量）"
        );
    }

    // Assert 3 — 外部条目稳定追加在内建之后（不是插入、不是重排）
    assert_eq!(
        observed_views.last().map(|view| view.name.as_str()),
        Some("mcp__x__y"),
        "外部工具必须稳定追加在内建之后，实际尾部 {:?}",
        observed_views.last().map(|view| view.name.clone())
    );

    // Assert 4 — 模型名不得重复（Run / Read 各一次），顺序与 registry 序一致
    let names: Vec<&str> = observed_views.iter().map(|view| view.name.as_str()).collect();
    let run_count = names.iter().filter(|name| **name == "Run").count();
    let read_count = names.iter().filter(|name| **name == "Read").count();
    assert_eq!(run_count, 1, "`Run` 在模型可见面必须恰好出现 1 次，实际 {run_count}：{names:?}");
    assert_eq!(read_count, 1, "`Read` 在模型可见面必须恰好出现 1 次，实际 {read_count}：{names:?}");
    let unique: BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "模型可见工具名不得重复，实际 {names:?}"
    );
}


// ─────────────────────────────────────────────────────────────────────────────────────────────
// 11. 授权口径（Lead 裁决）：外部来源必须尊重 `requires_approval`，内建行为不变
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn t1_external_requires_approval_is_ask_under_the_production_evaluator() {
    // Arrange — 外部 descriptor 显式声明 requires_approval；生产装的 GovernedPolicyEvaluator
    let mut descriptor = external_descriptor(
        "demo",
        "danger_tool",
        ToolKind::External,
        &[ToolPermissionScope::WorkspaceWrite],
    );
    descriptor.permission_declaration.requires_approval = true;
    let descriptor_id = descriptor.identity.descriptor_id.clone();
    let provider = Arc::new(RecordingProvider::new(
        "demo",
        vec![descriptor],
        ProviderBehavior::Return("must-not-run".to_string()),
    ));

    let dispatcher = GovernedDispatcher::new(
        Arc::new(builtin_snapshot()),
        Arc::new(FakeClock::new(0)),
    );
    dispatcher.register_policy_evaluator(Arc::new(GovernedPolicyEvaluator::default()));
    dispatcher
        .register_tool_provider(Arc::clone(&provider) as Arc<dyn ToolProvider>)
        .expect("外部 provider 注册必须成功");
    dispatcher.set_turn_view(dispatcher.registry().default_turn_tool_view());

    // Act
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &descriptor_id,
            "external-ask",
            json!({ "payload": "nope" }),
        ),
        &DispatchContext::default(),
    );

    // Assert 1 — 外部来源的 requires_approval 必须转成 Ask（控制面挂起），不得静默执行
    let control = outcome.control_outcome.as_ref().unwrap_or_else(|| {
        panic!(
            "外部 descriptor 声明 requires_approval=true 时，生产评估器必须给出审批请求（Ask），实际直接结束：{}",
            outcome_output(&outcome)
        )
    });
    assert_eq!(
        control.kind,
        pony_agent_core::agent::tools::ToolControlKind::ApprovalRequired,
        "外部来源的 requires_approval 必须映射为 ApprovalRequired（Ask）"
    );

    // Assert 2 — 审批前不得真正调用外部执行体
    assert!(
        provider.calls().is_empty(),
        "审批未完成前不得调用 provider，实际调用 {:?}",
        provider.calls()
    );
}

#[test]
fn t1_builtin_approval_behaviour_is_unchanged_under_the_production_evaluator() {
    // Arrange — 生产评估器 + 生命周期观察者
    let dispatcher = GovernedDispatcher::new(
        Arc::new(builtin_snapshot()),
        Arc::new(FakeClock::new(0)),
    );
    dispatcher.register_policy_evaluator(Arc::new(GovernedPolicyEvaluator::default()));
    let observer = Arc::new(RecordingObserver::default());
    dispatcher.register_lifecycle_observer(
        Arc::clone(&observer) as Arc<dyn DispatchLifecycleObserver>,
    );

    // Arrange — 用 descriptor 自己的 schema 生成最小合法入参，确保走到「授权判定」而不是
    // 停在 schema 校验（否则 decision 恒为 None，断言会空转）。
    let descriptor = dispatcher
        .registry()
        .descriptors
        .iter()
        .find(|descriptor| descriptor.identity.descriptor_id == "builtin:workspace_write_file")
        .expect("内建写工具必须在 registry 中")
        .clone();
    let arguments = minimal_arguments(&descriptor.input_schema);

    // Act — 内建写工具（未注册 handler，只看授权判定）
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &descriptor.identity.descriptor_id,
            "builtin-write-decision",
            arguments,
        ),
        &DispatchContext::default(),
    );

    // Assert — 内建工具的授权行为与今天一致：Allow（不得被「外部来源需审批」的分流顺手翻掉）
    let records = observer.records();
    assert_eq!(records.len(), 1, "必须恰好一条生命周期记录");
    assert_eq!(
        records[0].decision,
        Some(PermissionVerdict::Allow),
        "内建 `workspace_write_file` 在生产评估器下必须仍是 Allow（内建审批面无回归）"
    );
    assert!(
        outcome.control_outcome.is_none(),
        "内建写工具不得被改成审批挂起（内建行为不变量）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 12. 回归栅栏（L3 反例）：工具面撕裂 / 子派发钉代 / provider 泄漏
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// 返回固定标记的 handler，用于区分「同一 descriptor_id 的两代执行体」。
struct GenerationalHandler(&'static str);

impl PrimitiveToolHandler for GenerationalHandler {
    fn execute(&self, _request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        Ok(json!({ "handler": self.0 }))
    }
}

/// 在 pre-dispatch hook 里发布新一代工具面（B1 反例的触发器）。
struct PublishSurfaceHook {
    dispatcher: GovernedDispatcher,
    snapshot_id: String,
    registry: Mutex<Option<ToolRegistrySnapshot>>,
    handlers: Mutex<Option<Vec<(String, Arc<dyn PrimitiveToolHandler>)>>>,
    fired: AtomicUsize,
}

impl pony_agent_core::agent::dispatcher::PreDispatchHook for PublishSurfaceHook {
    fn rewrite(
        &self,
        _descriptor_id: &str,
        _origin: &InvocationOrigin,
        _call_id: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        if self.fired.swap(1, Ordering::SeqCst) == 0 {
            let registry = self
                .registry
                .lock()
                .expect("hook registry poisoned")
                .take()
                .expect("hook registry must be present exactly once");
            let handlers = self
                .handlers
                .lock()
                .expect("hook handlers poisoned")
                .take()
                .expect("hook handlers must be present exactly once");
            self.dispatcher
                .swap_tool_surface(&self.snapshot_id, registry, handlers)
                .map_err(|err| format!("hook swap failed: {err}"))?;
        }
        Ok(arguments)
    }
}

fn handlers_with_generation(
    registry: &ToolRegistrySnapshot,
    descriptor_id: &str,
    marker: &'static str,
) -> Vec<(String, Arc<dyn PrimitiveToolHandler>)> {
    coherent_handlers(registry)
        .into_iter()
        .map(|(id, handler)| {
            if id == descriptor_id {
                (id, Arc::new(GenerationalHandler(marker)) as Arc<dyn PrimitiveToolHandler>)
            } else {
                (id, handler)
            }
        })
        .collect()
}

#[test]
fn t1_surface_swap_inside_dispatch_keeps_authorization_and_execution_same_generation() {
    // Arrange — 同一 descriptor 的两代：gen1 只读、gen2 可写（destructive handler）
    let gen1_descriptor = external_descriptor(
        "demo",
        "tear_tool",
        ToolKind::External,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let descriptor_id = gen1_descriptor.identity.descriptor_id.clone();
    let gen2_descriptor = external_descriptor(
        "demo",
        "tear_tool",
        ToolKind::External,
        &[ToolPermissionScope::WorkspaceWrite],
    );
    let gen1 = registry_with("surface-gen1", vec![gen1_descriptor]);
    let gen2 = registry_with("surface-gen2", vec![gen2_descriptor]);
    let gen1_handlers = handlers_with_generation(&gen1, &descriptor_id, "GEN1_SAFE_HANDLER");
    let gen2_handlers = handlers_with_generation(&gen2, &descriptor_id, "GEN2_DESTRUCTIVE_HANDLER");

    let dispatcher = GovernedDispatcher::new(Arc::new(gen1), Arc::new(FakeClock::new(0)));
    dispatcher.register_policy_evaluator(Arc::new(GovernedPolicyEvaluator::default()));
    dispatcher.register_handler(
        descriptor_id.clone(),
        Arc::new(GenerationalHandler("GEN1_SAFE_HANDLER")) as Arc<dyn PrimitiveToolHandler>,
    );
    let hook = Arc::new(PublishSurfaceHook {
        dispatcher: dispatcher.clone(),
        snapshot_id: "surface-gen2".to_string(),
        registry: Mutex::new(Some(gen2)),
        handlers: Mutex::new(Some(gen2_handlers)),
        fired: AtomicUsize::new(0),
    });
    let _hook_keepalive = Arc::clone(&hook);
    dispatcher.register_pre_dispatch_hook(hook);
    dispatcher.set_turn_view(dispatcher.registry().default_turn_tool_view());
    let _ = gen1_handlers;

    // Act — 只读模式下发起调用；hook 在管道中途把工具面换到 gen2
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &descriptor_id,
            "tear-call",
            json!({ "payload": "x" }),
        ),
        &read_only_context(),
    );

    // Assert 1 — 破坏性 handler 绝不得执行（授权代与执行代必须同代）
    let output = outcome_output(&outcome);
    assert!(
        !output.contains("GEN2_DESTRUCTIVE_HANDLER"),
        "本次调用被 gen1 授权（只读）却执行了 gen2 的破坏性 handler —— 工具面撕裂（B1）。输出：{output}"
    );

    // Assert 2 — 若执行成功，则必须是 gen1 的执行体
    if outcome.execution_status == ToolExecutionStatus::Ok {
        assert!(
            output.contains("GEN1_SAFE_HANDLER"),
            "只读模式下成功执行时必须落在被授权的那一代执行体上，实际输出：{output}"
        );
    }
}

/// 在 composite 子派发前发布新代的 handler（B2 反例的触发器）。
struct PublishThenDispatchChild {
    dispatcher: GovernedDispatcher,
    snapshot_id: String,
    registry: Mutex<Option<ToolRegistrySnapshot>>,
    handlers: Mutex<Option<Vec<(String, Arc<dyn PrimitiveToolHandler>)>>>,
    child_descriptor_id: String,
}

impl pony_agent_core::agent::dispatcher::CompositeToolHandler for PublishThenDispatchChild {
    fn execute(
        &self,
        request: &pony_agent_core::agent::dispatcher::CompositeToolHandlerRequest,
        children: &pony_agent_core::agent::child_dispatch::ChildDispatch,
    ) -> Result<Value, String> {
        {
            let registry = self
                .registry
                .lock()
                .expect("composite registry poisoned")
                .take()
                .expect("composite registry must be present exactly once");
            let handlers = self
                .handlers
                .lock()
                .expect("composite handlers poisoned")
                .take()
                .expect("composite handlers must be present exactly once");
            self.dispatcher
                .swap_tool_surface(&self.snapshot_id, registry, handlers)?;
        }
        let child = children.dispatch(pony_agent_core::agent::child_dispatch::ChildDispatchRequest {
            descriptor_id: self.child_descriptor_id.clone(),
            call_id: format!("{}-child", request.call_id),
            arguments: json!({}),
        })?;
        Ok(json!({
            "child_status": child.execution_status.as_legacy_status(),
            "child_error": outcome_error_code(&child),
            "child_output": child.result.as_ref().map(|result| result.output.clone()),
        }))
    }
}

#[test]
fn t1_composite_child_dispatch_pins_authorization_and_handler_to_one_generation() {
    // Arrange — gen1: 复合工具 + 只读子工具；gen2: 同 id 子工具改成可写 + 破坏性 handler
    let child_gen1 = external_descriptor(
        "demo",
        "child_tool",
        ToolKind::External,
        &[ToolPermissionScope::WorkspaceRead],
    );
    let child_id = child_gen1.identity.descriptor_id.clone();
    let child_gen2 = external_descriptor(
        "demo",
        "child_tool",
        ToolKind::External,
        &[ToolPermissionScope::WorkspaceWrite],
    );
    let mut composite = external_descriptor(
        "demo",
        "batch_tool",
        ToolKind::Composite,
        &[ToolPermissionScope::WorkspaceRead],
    );
    composite.composed_descriptor_ids = vec![child_id.clone()];
    let composite_id = composite.identity.descriptor_id.clone();

    let gen1 = registry_with("surface-gen1", vec![child_gen1, composite]);
    let gen2 = registry_with("surface-gen2", vec![child_gen2]);
    let gen2_handlers = handlers_with_generation(&gen2, &child_id, "GEN2_DESTRUCTIVE_HANDLER");

    let dispatcher = GovernedDispatcher::new(Arc::new(gen1), Arc::new(FakeClock::new(0)));
    dispatcher.register_policy_evaluator(Arc::new(GovernedPolicyEvaluator::default()));
    dispatcher.register_handler(
        child_id.clone(),
        Arc::new(GenerationalHandler("GEN1_SAFE_HANDLER")) as Arc<dyn PrimitiveToolHandler>,
    );
    dispatcher.register_composite_handler(
        composite_id.clone(),
        Arc::new(PublishThenDispatchChild {
            dispatcher: dispatcher.clone(),
            snapshot_id: "surface-gen2".to_string(),
            registry: Mutex::new(Some(gen2)),
            handlers: Mutex::new(Some(gen2_handlers)),
            child_descriptor_id: child_id.clone(),
        }) as Arc<dyn pony_agent_core::agent::dispatcher::CompositeToolHandler>,
    );
    dispatcher.set_turn_view(dispatcher.registry().default_turn_tool_view());

    // Act
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &composite_id,
            "composite-tear",
            json!({}),
        ),
        &read_only_context(),
    );

    // Assert — 子派发不得「按 gen1 授权、跑 gen2 handler」
    let output = outcome_output(&outcome);
    assert!(
        !output.contains("GEN2_DESTRUCTIVE_HANDLER"),
        "子派发按 gen1 授权却执行了 gen2 的破坏性 handler —— 子派发未钉代（B2）。输出：{output}"
    );
    assert!(
        !output.contains("\"GEN2"),
        "子派发输出里出现了新一代执行体标记（B2）。输出：{output}"
    );
}

#[test]
fn t1_reregistering_a_source_prunes_the_previous_provider() {
    // Arrange — 同一 source_id 先注册两个工具，再用「只含一个工具」的第二代覆盖
    let first = Arc::new(RecordingProvider::new(
        "demo",
        vec![
            external_descriptor("demo", "tool_a", ToolKind::Read, &[ToolPermissionScope::WorkspaceRead]),
            external_descriptor("demo", "tool_b", ToolKind::Read, &[ToolPermissionScope::WorkspaceRead]),
        ],
        ProviderBehavior::Return("first-generation".to_string()),
    ));
    let dispatcher = GovernedDispatcher::new(
        Arc::new(builtin_snapshot()),
        Arc::new(FakeClock::new(0)),
    );
    dispatcher
        .register_tool_provider(Arc::clone(&first) as Arc<dyn ToolProvider>)
        .expect("首代 provider 注册必须成功");
    let first_weak = Arc::downgrade(&first);
    drop(first);
    // 每个 descriptor 绑定一个 `ExternalToolHandler`，各自持有一个 Arc ⇒ 首代计数 = 工具数（≥1）。
    let baseline_strong = first_weak.strong_count();
    assert!(
        baseline_strong >= 1,
        "首代 provider 必须仍被工具面持有（计数 ≥1），实际 {baseline_strong}"
    );

    // Act
    let second = Arc::new(RecordingProvider::new(
        "demo",
        vec![external_descriptor(
            "demo",
            "tool_a",
            ToolKind::Read,
            &[ToolPermissionScope::WorkspaceRead],
        )],
        ProviderBehavior::Return("second-generation".to_string()),
    ));
    dispatcher
        .register_tool_provider(Arc::clone(&second) as Arc<dyn ToolProvider>)
        .expect("二代 provider 覆盖注册必须成功");
    drop(second);

    // Assert 1 — 旧 provider 必须被剪除（只保留 1 个 live provider）
    assert_eq!(
        first_weak.strong_count(),
        0,
        "同一 source_id 二次注册后旧 provider 仍被持有（泄漏，B3），strong_count={}",
        first_weak.strong_count()
    );

    // Assert 2 — 新面里不存在的旧 descriptor 不得再可见
    let registry = dispatcher.registry();
    let stale = registry
        .descriptors
        .iter()
        .any(|descriptor| descriptor.identity.model_name == "mcp__demo__tool_b");
    assert!(
        !stale,
        "旧代 descriptor `mcp__demo__tool_b` 仍留在工具面（泄漏，B3）"
    );
    let fresh = registry
        .descriptors
        .iter()
        .any(|descriptor| descriptor.identity.model_name == "mcp__demo__tool_a");
    assert!(fresh, "新代 descriptor `mcp__demo__tool_a` 必须仍可见");
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// 13. 回归栅栏（第二轮 L3）：Draft 计划期的外部写工具必须与内建写工具同口径被拒
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn t1_draft_plan_denies_external_write_tool() {
    use pony_agent_core::agent::plan_state::{PlanPayload, PlanStore};

    // Arrange — 会话里存在 Draft 计划（写/执行类工具必须被拦）
    let plan_store = Arc::new(PlanStore::new());
    plan_store
        .create(
            "session-draft",
            PlanPayload {
                kind: "plan".to_string(),
                summary: "draft plan blocks write tools".to_string(),
                steps: Vec::new(),
            },
        )
        .expect("Draft 计划必须能创建");

    let external_write = external_descriptor(
        "demo",
        "ext_write_tool",
        ToolKind::External,
        &[ToolPermissionScope::WorkspaceWrite],
    );
    let external_id = external_write.identity.descriptor_id.clone();
    let invocations = Arc::new(AtomicUsize::new(0));
    let registry = Arc::new(registry_with("draft-surface", vec![external_write]));
    let dispatcher = GovernedDispatcher::new(Arc::clone(&registry), Arc::new(FakeClock::new(0)));
    dispatcher.register_policy_evaluator(Arc::new(GovernedPolicyEvaluator::new(
        Vec::new(),
        Some(Arc::clone(&plan_store)),
    )));
    dispatcher.register_handler(
        external_id.clone(),
        Arc::new(CountingHandler(Arc::clone(&invocations))) as Arc<dyn PrimitiveToolHandler>,
    );
    dispatcher.set_turn_view(registry.default_turn_tool_view());
    let context = DispatchContext {
        session_id: Some("session-draft".to_string()),
        ..Default::default()
    };

    // Act — 外部写工具在 Draft 计划期被调用
    let outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &external_id,
            "draft-external-write",
            json!({ "payload": "should be blocked" }),
        ),
        &context,
    );

    // Assert 1 — 必须 Deny（与内建 kind=Write 同口径），不是放行
    let code = outcome_error_code(&outcome);
    assert_eq!(
        code.as_deref(),
        Some("permission_denied"),
        "Draft 计划期外部写工具必须被 Deny（与内建写工具同口径），实际 {code:?}，输出：{}",
        outcome_output(&outcome)
    );

    // Assert 2 — 拒绝必须发生在执行之前（零副作用）
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        0,
        "Draft 计划期被拒绝的外部写工具不得执行 handler"
    );

    // Assert 3 — 内建写工具在同一上下文下的对照（证明"同口径"不是空的）
    let builtin = dispatcher
        .registry()
        .descriptors
        .iter()
        .find(|descriptor| descriptor.identity.descriptor_id == "builtin:workspace_write_file")
        .expect("内建写工具必须在 registry 中")
        .clone();
    let builtin_outcome = dispatcher.dispatch_governed(
        request(
            InvocationOrigin::Model,
            &builtin.identity.descriptor_id,
            "draft-builtin-write",
            minimal_arguments(&builtin.input_schema),
        ),
        &context,
    );
    assert_eq!(
        outcome_error_code(&builtin_outcome).as_deref(),
        Some("permission_denied"),
        "对照失败：内建写工具在 Draft 计划期必须同样 Deny，实际 {:?}",
        outcome_error_code(&builtin_outcome)
    );
}
