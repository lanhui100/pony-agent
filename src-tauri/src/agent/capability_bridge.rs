use crate::agent::tools::{builtin_tools, ToolCall, ToolDefinition, ToolResult};
use crate::agent::telemetry::CapabilityInvocationRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySourceKind {
    Builtin,
    Mcp,
}

impl CapabilitySourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Mcp => "mcp",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAvailability {
    Available,
    Degraded,
    Unreachable,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Tool,
    Resource,
    PromptTemplate,
}

impl CapabilityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Resource => "resource",
            Self::PromptTemplate => "prompt_template",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityInvocationMode {
    DirectToolCall,
    ReadOnlyFetch,
    PromptExpansion,
}

impl CapabilityInvocationMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DirectToolCall => "direct_tool_call",
            Self::ReadOnlyFetch => "read_only_fetch",
            Self::PromptExpansion => "prompt_expansion",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityFailureKind {
    SourceUnavailable,
    PermissionDenied,
    MalformedResponse,
    InvocationFailed,
    CapabilityNotFound,
}

impl CapabilityFailureKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SourceUnavailable => "source_unavailable",
            Self::PermissionDenied => "permission_denied",
            Self::MalformedResponse => "malformed_response",
            Self::InvocationFailed => "invocation_failed",
            Self::CapabilityNotFound => "capability_not_found",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySourceView {
    pub source_id: String,
    pub source_kind: CapabilitySourceKind,
    pub display_name: String,
    pub transport_kind: String,
    pub server_identity: String,
    pub availability: CapabilityAvailability,
    pub declared_capabilities: Vec<CapabilityKind>,
    pub permission_profile: String,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityView {
    pub capability_id: String,
    pub source_id: String,
    pub source_kind: CapabilitySourceKind,
    pub kind: CapabilityKind,
    pub label: String,
    pub description: String,
    pub invocation_mode: CapabilityInvocationMode,
    pub input_schema_summary: String,
    pub safety_class: String,
    pub visibility: String,
    pub observability_tags: Vec<String>,
    pub requires_approval: bool,
    pub host_mediated: bool,
    pub permission_scope: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSourceSnapshot {
    pub source: CapabilitySourceView,
    pub capabilities: Vec<CapabilityView>,
}

#[derive(Clone, Debug)]
pub struct CapabilityToolAction {
    pub capability: CapabilityView,
    pub tool_call: ToolCall,
}

#[derive(Clone, Debug)]
pub struct CapabilityResourceAction {
    pub capability: CapabilityView,
    pub arguments: Value,
}

#[derive(Clone, Debug)]
pub struct CapabilityPromptTemplateAction {
    pub capability: CapabilityView,
    pub arguments: Value,
}

#[derive(Clone, Debug)]
pub enum CapabilityBridgeAction {
    Tool(CapabilityToolAction),
    Resource(CapabilityResourceAction),
    PromptTemplate(CapabilityPromptTemplateAction),
}

#[derive(Clone, Debug)]
pub struct CapabilityInvocationRequest {
    pub capability_id: String,
    pub arguments: Value,
}

#[derive(Clone, Debug)]
pub struct CapabilityToolExecutionResult {
    pub capability: Option<CapabilityView>,
    pub tool_call: ToolCall,
    pub tool_result: ToolResult,
    pub failure_kind: Option<CapabilityFailureKind>,
}

#[derive(Clone, Debug)]
pub struct CapabilityResourceFetchResult {
    pub requested_capability_id: String,
    pub capability: Option<CapabilityView>,
    pub arguments: Value,
    pub content: Option<Value>,
    pub failure_kind: Option<CapabilityFailureKind>,
}

#[derive(Clone, Debug)]
pub struct CapabilityPromptExpansionResult {
    pub requested_capability_id: String,
    pub capability: Option<CapabilityView>,
    pub arguments: Value,
    pub prompt_text: Option<String>,
    pub failure_kind: Option<CapabilityFailureKind>,
}

impl CapabilityToolExecutionResult {
    pub fn invocation_record(&self) -> CapabilityInvocationRecord {
        CapabilityInvocationRecord {
            tool_name: self.tool_call.name.clone(),
            capability_id: self
                .capability
                .as_ref()
                .map(|capability| capability.capability_id.clone()),
            source_id: self
                .capability
                .as_ref()
                .map(|capability| capability.source_id.clone()),
            source_kind: self
                .capability
                .as_ref()
                .map(|capability| capability.source_kind.as_str().to_string()),
            capability_kind: self
                .capability
                .as_ref()
                .map(|capability| capability.kind.as_str().to_string()),
            invocation_mode: self
                .capability
                .as_ref()
                .map(|capability| capability.invocation_mode.as_str().to_string()),
            failure_kind: self.failure_kind.as_ref().map(|failure| failure.as_str().to_string()),
            requires_approval: self
                .capability
                .as_ref()
                .map(|capability| capability.requires_approval),
            host_mediated: self
                .capability
                .as_ref()
                .map(|capability| capability.host_mediated),
            permission_scope: self
                .capability
                .as_ref()
                .map(|capability| capability.permission_scope.clone()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CapabilityRegistry {
    sources: BTreeMap<String, CapabilitySourceView>,
    capabilities: BTreeMap<String, CapabilityView>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        let mut registry = Self::default();
        registry.register_builtin_source_and_capabilities();
        registry
    }

    pub fn list_sources(&self) -> Vec<CapabilitySourceView> {
        self.sources.values().cloned().collect()
    }

    pub fn list_capabilities(
        &self,
        source_id: Option<&str>,
        kind: Option<&str>,
    ) -> Vec<CapabilityView> {
        self.capabilities
            .values()
            .filter(|capability| match source_id {
                Some(source_id) => capability.source_id == source_id,
                None => true,
            })
            .filter(|capability| match kind {
                Some(kind) => capability.kind.as_str() == kind,
                None => true,
            })
            .cloned()
            .collect()
    }

    pub fn inspect_capability(&self, capability_id: &str) -> Option<CapabilityView> {
        self.capabilities.get(capability_id).cloned()
    }

    pub fn inspect_source(&self, source_id: &str) -> Option<CapabilitySourceView> {
        self.sources.get(source_id).cloned()
    }

    pub fn resolve_invocation(
        &self,
        request: &CapabilityInvocationRequest,
    ) -> Result<CapabilityBridgeAction, CapabilityFailureKind> {
        let capability = self
            .capabilities
            .get(&request.capability_id)
            .cloned()
            .ok_or(CapabilityFailureKind::CapabilityNotFound)?;
        let source = self
            .sources
            .get(&capability.source_id)
            .cloned()
            .ok_or(CapabilityFailureKind::MalformedResponse)?;

        if matches!(
            source.availability,
            CapabilityAvailability::Unreachable | CapabilityAvailability::Disabled
        ) {
            return Err(CapabilityFailureKind::SourceUnavailable);
        }

        if capability.requires_approval && !capability.host_mediated {
            return Err(CapabilityFailureKind::PermissionDenied);
        }

        match capability.kind {
            CapabilityKind::Tool => {
                let tool_name = capability.label.clone();
                Ok(CapabilityBridgeAction::Tool(CapabilityToolAction {
                    capability,
                    tool_call: ToolCall {
                        call_id: None,
                        name: tool_name,
                        arguments: request.arguments.clone(),
                        plan: None,
                    },
                }))
            }
            CapabilityKind::Resource => {
                Ok(CapabilityBridgeAction::Resource(CapabilityResourceAction {
                    capability,
                    arguments: request.arguments.clone(),
                }))
            }
            CapabilityKind::PromptTemplate => Ok(CapabilityBridgeAction::PromptTemplate(
                CapabilityPromptTemplateAction {
                    capability,
                    arguments: request.arguments.clone(),
                },
            )),
        }
    }

    pub fn resolve_tool_call(
        &self,
        tool_call: &ToolCall,
    ) -> Result<CapabilityToolAction, CapabilityFailureKind> {
        let action = candidate_builtin_capability_ids(&tool_call.name)
            .into_iter()
            .chain(self.capabilities.values().filter_map(|capability| {
                if capability.kind == CapabilityKind::Tool && capability.label == tool_call.name {
                    Some(capability.capability_id.clone())
                } else {
                    None
                }
            }))
            .find_map(|capability_id| {
                match self.resolve_invocation(&CapabilityInvocationRequest {
                    capability_id,
                    arguments: tool_call.arguments.clone(),
                }) {
                    Ok(action) => Some(Ok(action)),
                    Err(CapabilityFailureKind::CapabilityNotFound) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .unwrap_or(Err(CapabilityFailureKind::CapabilityNotFound))?;

        match action {
            CapabilityBridgeAction::Tool(action) => Ok(CapabilityToolAction {
                capability: action.capability,
                tool_call: tool_call.clone(),
            }),
            _ => Err(CapabilityFailureKind::MalformedResponse),
        }
    }

    pub fn capability_not_found_result(&self, tool_call: &ToolCall) -> CapabilityToolExecutionResult {
        self.capability_failure_result(tool_call, CapabilityFailureKind::CapabilityNotFound)
    }

    pub fn capability_failure_result(
        &self,
        tool_call: &ToolCall,
        failure_kind: CapabilityFailureKind,
    ) -> CapabilityToolExecutionResult {
        let output = match failure_kind {
            CapabilityFailureKind::CapabilityNotFound => {
                format!("未找到与工具 `{}` 对应的 capability registry 条目。", tool_call.name)
            }
            CapabilityFailureKind::SourceUnavailable => format!(
                "工具 `{}` 对应的 capability source 当前不可用。",
                tool_call.name
            ),
            CapabilityFailureKind::PermissionDenied => format!(
                "工具 `{}` 需要 host 审批或受管执行，但当前 capability 配置不满足该条件。",
                tool_call.name
            ),
            CapabilityFailureKind::MalformedResponse => format!(
                "工具 `{}` 对应的 capability registry 条目不完整或来源状态异常。",
                tool_call.name
            ),
            CapabilityFailureKind::InvocationFailed => {
                format!("工具 `{}` 的 capability 执行失败。", tool_call.name)
            }
        };
        CapabilityToolExecutionResult {
            capability: None,
            tool_call: tool_call.clone(),
            tool_result: ToolResult {
                tool_name: tool_call.name.clone(),
                status: "error".to_string(),
                output,
                duration_ms: 0,
            },
            failure_kind: Some(failure_kind),
        }
    }

    pub fn resource_fetch_success_result(
        &self,
        action: CapabilityResourceAction,
        content: Value,
    ) -> CapabilityResourceFetchResult {
        CapabilityResourceFetchResult {
            requested_capability_id: action.capability.capability_id.clone(),
            capability: Some(action.capability),
            arguments: action.arguments,
            content: Some(content),
            failure_kind: None,
        }
    }

    pub fn resource_fetch_failure_result(
        &self,
        request: &CapabilityInvocationRequest,
        failure_kind: CapabilityFailureKind,
    ) -> CapabilityResourceFetchResult {
        CapabilityResourceFetchResult {
            requested_capability_id: request.capability_id.clone(),
            capability: self.inspect_capability(&request.capability_id),
            arguments: request.arguments.clone(),
            content: None,
            failure_kind: Some(failure_kind),
        }
    }

    pub fn prompt_expansion_success_result(
        &self,
        action: CapabilityPromptTemplateAction,
        prompt_text: impl Into<String>,
    ) -> CapabilityPromptExpansionResult {
        CapabilityPromptExpansionResult {
            requested_capability_id: action.capability.capability_id.clone(),
            capability: Some(action.capability),
            arguments: action.arguments,
            prompt_text: Some(prompt_text.into()),
            failure_kind: None,
        }
    }

    pub fn prompt_expansion_failure_result(
        &self,
        request: &CapabilityInvocationRequest,
        failure_kind: CapabilityFailureKind,
    ) -> CapabilityPromptExpansionResult {
        CapabilityPromptExpansionResult {
            requested_capability_id: request.capability_id.clone(),
            capability: self.inspect_capability(&request.capability_id),
            arguments: request.arguments.clone(),
            prompt_text: None,
            failure_kind: Some(failure_kind),
        }
    }

    pub fn register_mcp_source(&mut self, source: CapabilitySourceView) {
        self.sources.insert(source.source_id.clone(), source);
    }

    #[cfg(test)]
    pub fn remove_source_for_test(&mut self, source_id: &str) {
        self.sources.remove(source_id);
    }

    pub fn replace_mcp_source_snapshot(&mut self, snapshot: McpSourceSnapshot) {
        let source_id = snapshot.source.source_id.clone();
        self.capabilities
            .retain(|_, capability| capability.source_id != source_id);

        let mut source = snapshot.source;
        for kind in snapshot.capabilities.iter().map(|capability| capability.kind.clone()) {
            if !source.declared_capabilities.iter().any(|declared| declared == &kind) {
                source.declared_capabilities.push(kind);
            }
        }
        self.register_mcp_source(source);

        for capability in snapshot.capabilities {
            self.register_mcp_capability(capability);
        }
    }

    pub fn register_mcp_capability(&mut self, capability: CapabilityView) {
        if let Some(source) = self.sources.get_mut(&capability.source_id) {
            if !source
                .declared_capabilities
                .iter()
                .any(|kind| kind == &capability.kind)
            {
                source.declared_capabilities.push(capability.kind.clone());
            }
            source.updated_at_ms = now_timestamp_ms();
        }
        self.capabilities
            .insert(capability.capability_id.clone(), capability);
    }

    pub fn register_mcp_tool_capability(
        &mut self,
        source_id: &str,
        capability_id: &str,
        label: &str,
        description: &str,
        input_schema_summary: &str,
        requires_approval: bool,
        permission_scope: &str,
    ) {
        self.register_mcp_capability(CapabilityView {
            capability_id: capability_id.to_string(),
            source_id: source_id.to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            kind: CapabilityKind::Tool,
            label: label.to_string(),
            description: description.to_string(),
            invocation_mode: CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: input_schema_summary.to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval,
            host_mediated: true,
            permission_scope: permission_scope.to_string(),
        });
    }

    pub fn register_mcp_resource_capability(
        &mut self,
        source_id: &str,
        capability_id: &str,
        label: &str,
        description: &str,
        input_schema_summary: &str,
        requires_approval: bool,
        permission_scope: &str,
    ) {
        self.register_mcp_capability(CapabilityView {
            capability_id: capability_id.to_string(),
            source_id: source_id.to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            kind: CapabilityKind::Resource,
            label: label.to_string(),
            description: description.to_string(),
            invocation_mode: CapabilityInvocationMode::ReadOnlyFetch,
            input_schema_summary: input_schema_summary.to_string(),
            safety_class: "read_only".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "resource".to_string()],
            requires_approval,
            host_mediated: true,
            permission_scope: permission_scope.to_string(),
        });
    }

    pub fn register_mcp_prompt_template_capability(
        &mut self,
        source_id: &str,
        capability_id: &str,
        label: &str,
        description: &str,
        input_schema_summary: &str,
        requires_approval: bool,
        permission_scope: &str,
    ) {
        self.register_mcp_capability(CapabilityView {
            capability_id: capability_id.to_string(),
            source_id: source_id.to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            kind: CapabilityKind::PromptTemplate,
            label: label.to_string(),
            description: description.to_string(),
            invocation_mode: CapabilityInvocationMode::PromptExpansion,
            input_schema_summary: input_schema_summary.to_string(),
            safety_class: "prompt_artifact".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "prompt_template".to_string()],
            requires_approval,
            host_mediated: true,
            permission_scope: permission_scope.to_string(),
        });
    }

    fn register_builtin_source_and_capabilities(&mut self) {
        let source_id = "builtin-tools".to_string();
        self.sources.insert(
            source_id.clone(),
            CapabilitySourceView {
                source_id: source_id.clone(),
                source_kind: CapabilitySourceKind::Builtin,
                display_name: "Builtin Tools".to_string(),
                transport_kind: "in_process".to_string(),
                server_identity: "pony-agent:builtin-tools".to_string(),
                availability: CapabilityAvailability::Available,
                declared_capabilities: vec![CapabilityKind::Tool],
                permission_profile: "host-mediated".to_string(),
                updated_at_ms: now_timestamp_ms(),
            },
        );

        for tool in builtin_tools() {
            let capability = capability_from_tool_definition(&source_id, &tool);
            self.capabilities
                .insert(capability.capability_id.clone(), capability);
        }
    }
}

fn capability_from_tool_definition(source_id: &str, tool: &ToolDefinition) -> CapabilityView {
    CapabilityView {
        capability_id: format!("builtin:{}", tool.name),
        source_id: source_id.to_string(),
        source_kind: CapabilitySourceKind::Builtin,
        kind: CapabilityKind::Tool,
        label: tool.name.to_string(),
        description: tool.description.to_string(),
        invocation_mode: CapabilityInvocationMode::DirectToolCall,
        input_schema_summary: summarize_input_schema(&tool.input_schema),
        safety_class: "host_tool".to_string(),
        visibility: "default".to_string(),
        observability_tags: vec!["builtin".to_string(), "tool".to_string()],
        requires_approval: false,
        host_mediated: true,
        permission_scope: "workspace".to_string(),
    }
}

fn candidate_builtin_capability_ids(tool_name: &str) -> Vec<String> {
    let mut candidates = Vec::with_capacity(2);
    let raw = tool_name.trim();
    if raw.is_empty() {
        return candidates;
    }

    candidates.push(format!("builtin:{raw}"));

    let canonical = raw.replace('.', "_");
    if canonical != raw {
        candidates.push(format!("builtin:{canonical}"));
    }

    candidates
}

fn summarize_input_schema(value: &Value) -> String {
    let schema_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("object");
    let properties = value
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| {
            let mut keys = properties.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            keys.join(", ")
        })
        .unwrap_or_default();

    if properties.is_empty() {
        schema_type.to_string()
    } else {
        format!("{schema_type}: {properties}")
    }
}

fn now_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_exposes_builtin_tools_as_normalized_capabilities() {
        let registry = CapabilityRegistry::new();

        let sources = registry.list_sources();
        assert!(sources.iter().any(|source| source.source_id == "builtin-tools"));

        let capabilities = registry.list_capabilities(Some("builtin-tools"), Some("tool"));
        assert!(!capabilities.is_empty());
        assert!(capabilities
            .iter()
            .any(|capability| capability.capability_id == "builtin:time_now"));
    }

    #[test]
    fn registry_can_hold_mcp_sources_and_capabilities() {
        let mut registry = CapabilityRegistry::new();
        registry.register_mcp_source(CapabilitySourceView {
            source_id: "mcp-local".to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            display_name: "Local MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "local/server".to_string(),
            availability: CapabilityAvailability::Degraded,
            declared_capabilities: vec![CapabilityKind::Resource, CapabilityKind::Tool],
            permission_profile: "requires-approval".to_string(),
            updated_at_ms: 1,
        });
        registry.register_mcp_capability(CapabilityView {
            capability_id: "mcp:resource:workspace-docs".to_string(),
            source_id: "mcp-local".to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            kind: CapabilityKind::Resource,
            label: "workspace-docs".to_string(),
            description: "Workspace docs".to_string(),
            invocation_mode: CapabilityInvocationMode::ReadOnlyFetch,
            input_schema_summary: "object: path".to_string(),
            safety_class: "read_only".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "resource".to_string()],
            requires_approval: true,
            host_mediated: true,
            permission_scope: "workspace:read".to_string(),
        });

        let capability = registry
            .inspect_capability("mcp:resource:workspace-docs")
            .expect("mcp capability should exist");
        assert_eq!(capability.kind.as_str(), "resource");
        assert!(capability.requires_approval);
        assert_eq!(capability.permission_scope, "workspace:read");
    }

    #[test]
    fn registry_resolves_builtin_tool_calls_for_dotted_and_canonical_names() {
        let registry = CapabilityRegistry::new();

        let canonical = registry
            .resolve_tool_call(&ToolCall {
                call_id: None,
                name: "time_now".to_string(),
                arguments: Value::Object(Default::default()),
                plan: None,
            })
            .expect("canonical tool call should resolve");
        assert_eq!(canonical.capability.capability_id, "builtin:time_now");

        let dotted = registry
            .resolve_tool_call(&ToolCall {
                call_id: None,
                name: "time.now".to_string(),
                arguments: Value::Object(Default::default()),
                plan: None,
            })
            .expect("dotted tool call should resolve");
        assert_eq!(dotted.capability.capability_id, "builtin:time_now");
    }

    #[test]
    fn registry_resolves_mcp_tool_resource_and_prompt_template_actions() {
        let mut registry = CapabilityRegistry::new();
        registry.register_mcp_source(CapabilitySourceView {
            source_id: "mcp-local".to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            display_name: "Local MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "local/server".to_string(),
            availability: CapabilityAvailability::Available,
            declared_capabilities: vec![
                CapabilityKind::Tool,
                CapabilityKind::Resource,
                CapabilityKind::PromptTemplate,
            ],
            permission_profile: "mixed".to_string(),
            updated_at_ms: 1,
        });
        registry.register_mcp_tool_capability(
            "mcp-local",
            "mcp:tool:list-files",
            "list_files",
            "List files",
            "object: path",
            true,
            "workspace:read",
        );
        registry.register_mcp_resource_capability(
            "mcp-local",
            "mcp:resource:repo-index",
            "repo_index",
            "Repository index",
            "object: path",
            false,
            "workspace:read",
        );
        registry.register_mcp_prompt_template_capability(
            "mcp-local",
            "mcp:prompt_template:review",
            "review_template",
            "Review prompt",
            "object: topic",
            false,
            "prompt:expand",
        );

        let tool_action = registry
            .resolve_invocation(&CapabilityInvocationRequest {
                capability_id: "mcp:tool:list-files".to_string(),
                arguments: serde_json::json!({ "path": "." }),
            })
            .expect("tool action should resolve");
        assert!(matches!(tool_action, CapabilityBridgeAction::Tool(_)));

        let resource_action = registry
            .resolve_invocation(&CapabilityInvocationRequest {
                capability_id: "mcp:resource:repo-index".to_string(),
                arguments: serde_json::json!({ "path": "src" }),
            })
            .expect("resource action should resolve");
        assert!(matches!(resource_action, CapabilityBridgeAction::Resource(_)));

        let prompt_action = registry
            .resolve_invocation(&CapabilityInvocationRequest {
                capability_id: "mcp:prompt_template:review".to_string(),
                arguments: serde_json::json!({ "topic": "PA-020" }),
            })
            .expect("prompt action should resolve");
        assert!(matches!(
            prompt_action,
            CapabilityBridgeAction::PromptTemplate(_)
        ));
    }

    #[test]
    fn registry_normalizes_source_unavailable_and_permission_denied_failures() {
        let mut registry = CapabilityRegistry::new();
        registry.register_mcp_source(CapabilitySourceView {
            source_id: "mcp-unavailable".to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            display_name: "Unavailable MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "local/unavailable".to_string(),
            availability: CapabilityAvailability::Unreachable,
            declared_capabilities: vec![CapabilityKind::Tool],
            permission_profile: "requires-approval".to_string(),
            updated_at_ms: 1,
        });
        registry.register_mcp_tool_capability(
            "mcp-unavailable",
            "mcp:tool:blocked",
            "blocked_tool",
            "Blocked tool",
            "object",
            false,
            "workspace:read",
        );
        let unavailable = registry.resolve_invocation(&CapabilityInvocationRequest {
            capability_id: "mcp:tool:blocked".to_string(),
            arguments: serde_json::json!({}),
        });
        assert!(matches!(
            unavailable,
            Err(CapabilityFailureKind::SourceUnavailable)
        ));

        registry.register_mcp_source(CapabilitySourceView {
            source_id: "mcp-no-host".to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            display_name: "No Host MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "local/no-host".to_string(),
            availability: CapabilityAvailability::Available,
            declared_capabilities: vec![CapabilityKind::Tool],
            permission_profile: "requires-approval".to_string(),
            updated_at_ms: 1,
        });
        registry.register_mcp_capability(CapabilityView {
            capability_id: "mcp:tool:needs-host".to_string(),
            source_id: "mcp-no-host".to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            kind: CapabilityKind::Tool,
            label: "needs_host".to_string(),
            description: "Needs host approval".to_string(),
            invocation_mode: CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "object".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: true,
            host_mediated: false,
            permission_scope: "workspace:write".to_string(),
        });
        let permission_denied = registry.resolve_invocation(&CapabilityInvocationRequest {
            capability_id: "mcp:tool:needs-host".to_string(),
            arguments: serde_json::json!({}),
        });
        assert!(matches!(
            permission_denied,
            Err(CapabilityFailureKind::PermissionDenied)
        ));
    }

    #[test]
    fn registry_builds_normalized_resource_fetch_results() {
        let mut registry = CapabilityRegistry::new();
        registry.register_mcp_source(CapabilitySourceView {
            source_id: "mcp-resource".to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            display_name: "Resource MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "local/resource".to_string(),
            availability: CapabilityAvailability::Available,
            declared_capabilities: vec![CapabilityKind::Resource],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
        });
        registry.register_mcp_resource_capability(
            "mcp-resource",
            "mcp:resource:repo-index",
            "repo_index",
            "Repository index",
            "object",
            false,
            "workspace:read",
        );

        let request = CapabilityInvocationRequest {
            capability_id: "mcp:resource:repo-index".to_string(),
            arguments: serde_json::json!({ "path": "src" }),
        };
        let action = registry
            .resolve_invocation(&request)
            .expect("resource action should resolve");
        let CapabilityBridgeAction::Resource(action) = action else {
            panic!("expected resource action");
        };

        let success = registry.resource_fetch_success_result(
            action,
            serde_json::json!({ "entries": ["src/main.rs"] }),
        );
        assert_eq!(
            success.requested_capability_id,
            "mcp:resource:repo-index".to_string()
        );
        assert_eq!(
            success
                .capability
                .as_ref()
                .map(|capability| capability.kind.as_str()),
            Some("resource")
        );
        assert_eq!(success.arguments, serde_json::json!({ "path": "src" }));
        assert_eq!(
            success.content,
            Some(serde_json::json!({ "entries": ["src/main.rs"] }))
        );
        assert_eq!(success.failure_kind, None);

        let failed_request = CapabilityInvocationRequest {
            capability_id: "mcp:resource:missing".to_string(),
            arguments: serde_json::json!({ "path": "src" }),
        };
        let failure = registry.resource_fetch_failure_result(
            &failed_request,
            CapabilityFailureKind::CapabilityNotFound,
        );
        assert_eq!(
            failure.requested_capability_id,
            "mcp:resource:missing".to_string()
        );
        assert!(failure.capability.is_none());
        assert_eq!(
            failure.failure_kind,
            Some(CapabilityFailureKind::CapabilityNotFound)
        );
    }

    #[test]
    fn registry_builds_normalized_prompt_expansion_results() {
        let mut registry = CapabilityRegistry::new();
        registry.register_mcp_source(CapabilitySourceView {
            source_id: "mcp-prompt".to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            display_name: "Prompt MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "local/prompt".to_string(),
            availability: CapabilityAvailability::Available,
            declared_capabilities: vec![CapabilityKind::PromptTemplate],
            permission_profile: "host-mediated".to_string(),
            updated_at_ms: 1,
        });
        registry.register_mcp_prompt_template_capability(
            "mcp-prompt",
            "mcp:prompt_template:review",
            "review_template",
            "Review template",
            "object",
            false,
            "prompt:expand",
        );

        let request = CapabilityInvocationRequest {
            capability_id: "mcp:prompt_template:review".to_string(),
            arguments: serde_json::json!({ "topic": "PA-020" }),
        };
        let action = registry
            .resolve_invocation(&request)
            .expect("prompt action should resolve");
        let CapabilityBridgeAction::PromptTemplate(action) = action else {
            panic!("expected prompt action");
        };

        let success = registry.prompt_expansion_success_result(
            action,
            "请审查 PA-020 的 capability bridge 边界。",
        );
        assert_eq!(
            success.requested_capability_id,
            "mcp:prompt_template:review".to_string()
        );
        assert_eq!(
            success
                .capability
                .as_ref()
                .map(|capability| capability.kind.as_str()),
            Some("prompt_template")
        );
        assert_eq!(success.arguments, serde_json::json!({ "topic": "PA-020" }));
        assert_eq!(
            success.prompt_text.as_deref(),
            Some("请审查 PA-020 的 capability bridge 边界。")
        );
        assert_eq!(success.failure_kind, None);

        registry.register_mcp_source(CapabilitySourceView {
            source_id: "mcp-prompt-denied".to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            display_name: "Denied Prompt MCP".to_string(),
            transport_kind: "stdio".to_string(),
            server_identity: "local/prompt-denied".to_string(),
            availability: CapabilityAvailability::Available,
            declared_capabilities: vec![CapabilityKind::PromptTemplate],
            permission_profile: "requires-approval".to_string(),
            updated_at_ms: 1,
        });
        registry.register_mcp_capability(CapabilityView {
            capability_id: "mcp:prompt_template:guarded".to_string(),
            source_id: "mcp-prompt-denied".to_string(),
            source_kind: CapabilitySourceKind::Mcp,
            kind: CapabilityKind::PromptTemplate,
            label: "guarded_template".to_string(),
            description: "Guarded prompt".to_string(),
            invocation_mode: CapabilityInvocationMode::PromptExpansion,
            input_schema_summary: "object".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "prompt_template".to_string()],
            requires_approval: true,
            host_mediated: false,
            permission_scope: "prompt:expand".to_string(),
        });
        let failure_request = CapabilityInvocationRequest {
            capability_id: "mcp:prompt_template:guarded".to_string(),
            arguments: serde_json::json!({ "topic": "PA-020" }),
        };
        let failure = registry.prompt_expansion_failure_result(
            &failure_request,
            CapabilityFailureKind::PermissionDenied,
        );
        assert_eq!(
            failure
                .capability
                .as_ref()
                .map(|capability| capability.capability_id.as_str()),
            Some("mcp:prompt_template:guarded")
        );
        assert_eq!(
            failure.failure_kind,
            Some(CapabilityFailureKind::PermissionDenied)
        );
        assert!(failure.prompt_text.is_none());
    }
}
