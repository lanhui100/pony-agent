//! 工具契约元数据：`ToolKind`/`ToolExposure`/权限声明/描述符/注册表快照/`ToolSurface`，
//! 以及描述符自校验。纯数据与校验，无 IO。
use super::*;
use super::names::builtin_aliases_for_primitive;
use super::names::default_permission_declaration_for_name;
use super::names::execution_policy_for_primitive;
use super::run::DEFAULT_TOOL_TIMEOUT_MS;
use super::schema::contract_priority;
use super::workspace::MAX_FULL_READ_BYTES;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Search,
    Write,
    Execute,
    BatchExecute,
    Interactive,
    Composite,
    External,
}

impl ToolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Search => "search",
            Self::Write => "write",
            Self::Execute => "execute",
            Self::BatchExecute => "batch_execute",
            Self::Interactive => "interactive",
            Self::Composite => "composite",
            Self::External => "external",
        }
    }
}


#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolExposure {
    ModelVisible,
    Internal,
    Deferred,
}

impl ToolExposure {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ModelVisible => "model_visible",
            Self::Internal => "internal",
            Self::Deferred => "deferred",
        }
    }
}


#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolDisplayMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name_zh: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolPermissionFacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_approval: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_mediated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_source: Option<String>,
}

/// Typed permission facts used for registry and dispatcher decisions. The string-shaped
/// `ToolPermissionFacts` remains a contract-view adapter for legacy consumers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionScope {
    WorkspaceRead,
    WorkspaceWrite,
    WorkspaceExecute,
    CapabilityDiscovery,
}

impl ToolPermissionScope {
    fn as_legacy_scope(&self) -> &'static str {
        match self {
            Self::WorkspaceRead => "workspace.read",
            Self::WorkspaceWrite => "workspace.write",
            Self::WorkspaceExecute => "workspace.execute",
            Self::CapabilityDiscovery => "capability.discovery",
        }
    }
}


#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolPermissionDeclaration {
    #[serde(default)]
    pub scopes: std::collections::BTreeSet<ToolPermissionScope>,
    #[serde(default)]
    pub requires_approval: bool,
    #[serde(default)]
    pub host_mediated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<String>,
}

impl ToolPermissionDeclaration {
    pub fn contract_facts(&self) -> ToolPermissionFacts {
        ToolPermissionFacts {
            requires_approval: Some(self.requires_approval),
            permission_scope: (!self.scopes.is_empty()).then(|| {
                self.scopes
                    .iter()
                    .map(ToolPermissionScope::as_legacy_scope)
                    .collect::<Vec<_>>()
                    .join(" + ")
            }),
            host_mediated: Some(self.host_mediated),
            permission_profile: None,
            approval_mode: self.approval_mode.clone(),
            decision_source: None,
        }
    }
}


#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolError {
    pub kind: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinitionContractView {
    pub name: String,
    pub canonical_tool_name: String,
    pub execution_primitive: String,
    pub description: String,
    pub input_schema: Value,
    pub kind: String,
    pub exposure: String,
    pub display_metadata: ToolDisplayMetadata,
    pub permission_facts: ToolPermissionFacts,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolDescriptorSource {
    Builtin,
    Mcp,
    Skill,
    Dynamic,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolIdentity {
    pub descriptor_id: String,
    pub model_name: String,
    pub canonical_name: String,
    pub primitive_name: String,
    pub source: ToolDescriptorSource,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionPolicy {
    pub concurrent_safe: bool,
    pub cancellable: bool,
    pub default_timeout_ms: u64,
    pub result_budget_bytes: usize,
}

impl Default for ToolExecutionPolicy {
    fn default() -> Self {
        Self {
            concurrent_safe: false,
            cancellable: true,
            // 通用工具默认 deadline 与 Run 默认超时解耦（契约只覆盖 Run）。
            default_timeout_ms: DEFAULT_TOOL_TIMEOUT_MS,
            result_budget_bytes: MAX_FULL_READ_BYTES as usize,
        }
    }
}


#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolHandlerProvenance {
    pub handler_kind: String,
    pub source_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDescriptor {
    pub identity: ToolIdentity,
    pub aliases: Vec<String>,
    pub description: String,
    pub input_schema: Value,
    pub kind: ToolKind,
    pub exposure: ToolExposure,
    pub permission_declaration: ToolPermissionDeclaration,
    pub execution_policy: ToolExecutionPolicy,
    pub display_metadata: ToolDisplayMetadata,
    pub handler_provenance: ToolHandlerProvenance,
    pub source_revision: String,
    #[serde(default)]
    pub composed_descriptor_ids: Vec<String>,
}

impl ToolDescriptor {
    pub fn contract_view(&self) -> ToolDefinitionContractView {
        ToolDefinitionContractView {
            name: self.identity.model_name.clone(),
            canonical_tool_name: self.identity.canonical_name.clone(),
            execution_primitive: self.identity.primitive_name.clone(),
            description: self.description.clone(),
            input_schema: self.input_schema.clone(),
            kind: self.kind.as_str().to_string(),
            exposure: self.exposure.as_str().to_string(),
            display_metadata: self.display_metadata.clone(),
            permission_facts: self.permission_declaration.contract_facts(),
        }
    }
}


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRegistrySnapshot {
    pub snapshot_id: String,
    pub descriptors: Vec<ToolDescriptor>,
    aliases: std::collections::BTreeMap<String, usize>,
}

impl ToolRegistrySnapshot {
    pub fn builtin() -> Result<Self, String> {
        Self::from_builtin_definitions(builtin_tools())
    }

    pub fn from_builtin_definitions(definitions: Vec<ToolDefinition>) -> Result<Self, String> {
        let mut product_winners: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        let mut product_priorities: std::collections::BTreeMap<String, u8> =
            std::collections::BTreeMap::new();
        // The provider tool array order is part of the cache-friendly stable prefix, so a product
        // name keeps the slot where it first appears even when a later primitive wins exposure.
        let mut product_first_index: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();

        for (index, definition) in definitions.iter().enumerate() {
            let contract = definition.contract_view();
            let priority = contract_priority(&contract);
            product_first_index
                .entry(contract.name.clone())
                .or_insert(index);
            let replace = product_priorities
                .get(&contract.name)
                .map(|existing| priority > *existing)
                .unwrap_or(true);
            if replace {
                product_priorities.insert(contract.name.clone(), priority);
                product_winners.insert(contract.name, contract.execution_primitive);
            }
        }

        let mut descriptors = Vec::with_capacity(definitions.len());
        for definition in definitions {
            let contract = definition.contract_view();
            let primitive = contract.execution_primitive.clone();
            let model_visible = product_winners
                .get(&contract.name)
                .map(|winner| winner == &primitive)
                .unwrap_or(false);
            let exposure = if model_visible {
                tool_exposure_for_name(&primitive)
            } else if primitive == TOOL_WORKSPACE_PATH_INFO {
                ToolExposure::Deferred
            } else {
                ToolExposure::Internal
            };
            let mut aliases = builtin_aliases_for_primitive(&primitive)
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>();
            if model_visible {
                aliases.push(contract.name.clone());
            }
            aliases.sort();
            aliases.dedup();

            descriptors.push(ToolDescriptor {
                identity: ToolIdentity {
                    descriptor_id: format!("builtin:{primitive}"),
                    model_name: contract.name,
                    canonical_name: contract.canonical_tool_name,
                    primitive_name: primitive.clone(),
                    source: ToolDescriptorSource::Builtin,
                },
                aliases,
                description: definition.description.to_string(),
                input_schema: definition.input_schema,
                kind: tool_kind_for_name(&primitive),
                exposure,
                permission_declaration: default_permission_declaration_for_name(&primitive),
                execution_policy: execution_policy_for_primitive(&primitive),
                display_metadata: tool_display_metadata_for_name(&primitive),
                handler_provenance: ToolHandlerProvenance {
                    handler_kind: "tool_router".to_string(),
                    source_id: "builtin-tools".to_string(),
                },
                source_revision: "builtin-tool-catalog-v1".to_string(),
                composed_descriptor_ids: Vec::new(),
            });
        }

        // Order descriptors so registry position is the single truth source for every projection:
        // model-visible descriptors keep their product name's first-appearance slot, and the
        // remaining internal/deferred descriptors follow in stable definition order.
        descriptors.sort_by_key(|descriptor| {
            let visible = descriptor.exposure != ToolExposure::Internal
                && product_winners
                    .get(&descriptor.identity.model_name)
                    .map(|winner| winner == &descriptor.identity.primitive_name)
                    .unwrap_or(false);
            let slot = product_first_index
                .get(&descriptor.identity.model_name)
                .copied()
                .unwrap_or(usize::MAX);
            (!visible, slot)
        });

        Self::from_descriptors("builtin-tool-catalog-v1", descriptors)
    }

    pub fn from_descriptors(
        snapshot_id: impl Into<String>,
        descriptors: Vec<ToolDescriptor>,
    ) -> Result<Self, String> {
        let snapshot_id = snapshot_id.into();
        if snapshot_id.trim().is_empty() {
            return Err("tool registry snapshot id cannot be empty".to_string());
        }
        let mut descriptor_ids = std::collections::BTreeSet::new();
        let mut aliases = std::collections::BTreeMap::new();
        for (index, descriptor) in descriptors.iter().enumerate() {
            validate_descriptor_identity(descriptor)?;
            if !descriptor_ids.insert(descriptor.identity.descriptor_id.clone()) {
                return Err(format!(
                    "tool registry contains duplicate descriptor id `{}`",
                    descriptor.identity.descriptor_id
                ));
            }
            for alias in &descriptor.aliases {
                let normalized = alias.trim();
                if normalized.is_empty() {
                    return Err(format!(
                        "tool registry descriptor `{}` contains an empty alias",
                        descriptor.identity.descriptor_id
                    ));
                }
                if let Some(existing_index) = aliases.insert(normalized.to_string(), index) {
                    let existing = &descriptors[existing_index].identity.descriptor_id;
                    return Err(format!(
                        "tool registry alias `{normalized}` is ambiguous between `{existing}` and `{}`",
                        descriptor.identity.descriptor_id
                    ));
                }
            }
        }

        validate_composite_dependencies(&descriptors, &descriptor_ids)?;

        Ok(Self {
            snapshot_id,
            descriptors,
            aliases,
        })
    }

    pub fn resolve(&self, raw_name: &str) -> Option<&ToolDescriptor> {
        self.aliases
            .get(raw_name.trim())
            .and_then(|index| self.descriptors.get(*index))
    }

    pub fn provider_contract_views(&self) -> Vec<ToolDefinitionContractView> {
        self.descriptors
            .iter()
            // ToolSearch is the one default deferred discovery entry. It remains advertised
            // during the migration so the model can request a governed elevation.
            .filter(|descriptor| {
                descriptor.exposure == ToolExposure::ModelVisible
                    || descriptor.identity.primitive_name == TOOL_TOOL_SEARCH
            })
            .map(ToolDescriptor::contract_view)
            .collect()
    }

    pub fn default_turn_tool_view(&self) -> TurnToolView {
        TurnToolView::from_registry(self)
    }

    pub fn model_name_for_primitive(&self, primitive_name: &str) -> Option<&str> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.identity.primitive_name == primitive_name)
            .map(|descriptor| descriptor.identity.model_name.as_str())
    }

    pub fn replace_source(
        &self,
        source_id: &str,
        replacement_snapshot_id: impl Into<String>,
        replacement: Vec<ToolDescriptor>,
    ) -> Result<Self, String> {
        if source_id.trim().is_empty() {
            return Err("tool registry source id cannot be empty".to_string());
        }
        if replacement
            .iter()
            .any(|descriptor| descriptor.handler_provenance.source_id != source_id)
        {
            return Err(
                "replacement descriptors must all belong to the replaced source".to_string(),
            );
        }
        if self.descriptors.iter().any(|descriptor| {
            descriptor.handler_provenance.source_id == source_id
                && descriptor.identity.source == ToolDescriptorSource::Builtin
        }) {
            return Err(
                "builtin registry descriptors cannot be replaced by an external source".to_string(),
            );
        }

        let mut next = self
            .descriptors
            .iter()
            .filter(|descriptor| descriptor.handler_provenance.source_id != source_id)
            .cloned()
            .collect::<Vec<_>>();
        next.extend(replacement);
        Self::from_descriptors(replacement_snapshot_id, next)
    }
}


/// Structured failure from an external [`ToolProvider`]. The `code` is carried verbatim into the
/// governed dispatcher's `error.code` so callers can distinguish failure sources
/// (`mcp_server_unavailable` / `mcp_tool_not_found` / `mcp_timeout` / `plugin_unavailable`, …)
/// instead of collapsing every external failure into `handler_error` (ADR 0065 decision 9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolProviderError {
    pub code: String,
    pub message: String,
}

impl ToolProviderError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// `code: message` shape the dispatcher's `handler_failure` lifts back into `error.code`.
    pub fn into_handler_message(self) -> String {
        format!("{}: {}", self.code, self.message)
    }
}


/// In-process abstract of an external tool source (ADR 0065 decision 6, the L1 extension seam).
///
/// MCP servers (ADR 0066) and the plugin host (ADR 0067) are both implementations of this trait —
/// **the one and only** external tool channel: an external capability must never bypass the
/// governed dispatcher pipeline (authorization, budget reservation, lifecycle evidence).
///
/// Contract:
/// - `list_tools` returns descriptors owned by `source_id`; the registry rejects descriptors that
///   claim another source, a builtin namespace, or a reserved builtin alias.
/// - `call` receives the stable `descriptor_id` (not the model-visible name), so model-name or
///   alias shapes never become dispatch truth.
pub trait ToolProvider: Send + Sync {
    fn source_id(&self) -> &str;
    fn list_tools(&self) -> Vec<ToolDescriptor>;
    fn call(&self, descriptor_id: &str, arguments: Value) -> Result<Value, ToolProviderError>;
}

#[derive(Clone, Debug)]
pub struct ToolSurface {
    pub registry: ToolRegistrySnapshot,
    pub turn_view: TurnToolView,
}

impl ToolSurface {
    pub fn builtin() -> Result<Self, String> {
        let registry = ToolRegistrySnapshot::builtin()?;
        let turn_view = registry.default_turn_tool_view();
        Ok(Self {
            registry,
            turn_view,
        })
    }

    pub fn provider_contract_views(&self) -> Result<Vec<ToolDefinitionContractView>, String> {
        self.turn_view.provider_contract_views(&self.registry)
    }

    pub fn model_name_for_primitive(&self, primitive_name: &str) -> Option<&str> {
        self.registry.model_name_for_primitive(primitive_name)
    }
}


pub fn builtin_tool_surface() -> ToolSurface {
    ToolSurface::builtin().expect("static builtin tool surface must validate")
}

pub fn builtin_turn_tool_contract_views() -> Vec<ToolDefinitionContractView> {
    builtin_tool_surface()
        .provider_contract_views()
        .expect("builtin turn tool view must match registry snapshot")
}

pub(super) fn validate_descriptor_identity(descriptor: &ToolDescriptor) -> Result<(), String> {
    let identity = &descriptor.identity;
    if identity.descriptor_id.trim().is_empty()
        || identity.model_name.trim().is_empty()
        || identity.canonical_name.trim().is_empty()
        || identity.primitive_name.trim().is_empty()
    {
        return Err("tool registry descriptor identity fields cannot be empty".to_string());
    }
    if descriptor.handler_provenance.handler_kind.trim().is_empty()
        || descriptor.handler_provenance.source_id.trim().is_empty()
        || descriptor.source_revision.trim().is_empty()
    {
        return Err(format!(
            "tool registry descriptor `{}` has incomplete source provenance",
            identity.descriptor_id
        ));
    }

    let expected_prefix = match identity.source {
        ToolDescriptorSource::Builtin => "builtin:",
        ToolDescriptorSource::Mcp => "mcp:",
        ToolDescriptorSource::Skill => "skill:",
        ToolDescriptorSource::Dynamic => "dynamic:",
    };
    if !identity.descriptor_id.starts_with(expected_prefix) {
        return Err(format!(
            "tool registry descriptor `{}` does not match declared source namespace `{expected_prefix}`",
            identity.descriptor_id
        ));
    }
    if identity.source == ToolDescriptorSource::Builtin
        && descriptor.handler_provenance.source_id != "builtin-tools"
    {
        return Err(format!(
            "builtin descriptor `{}` must use the builtin-tools provenance source",
            identity.descriptor_id
        ));
    }
    if identity.source != ToolDescriptorSource::Builtin
        && descriptor.handler_provenance.source_id == "builtin-tools"
    {
        return Err(format!(
            "non-builtin descriptor `{}` cannot claim builtin-tools provenance",
            identity.descriptor_id
        ));
    }
    if matches!(
        identity.source,
        ToolDescriptorSource::Mcp | ToolDescriptorSource::Skill
    ) {
        let source_prefix = format!(
            "{}{}:",
            expected_prefix, descriptor.handler_provenance.source_id
        );
        if !identity.descriptor_id.starts_with(&source_prefix) {
            return Err(format!(
                "descriptor `{}` does not belong to provenance source `{}`",
                identity.descriptor_id, descriptor.handler_provenance.source_id
            ));
        }
    }
    if identity.source != ToolDescriptorSource::Builtin
        && descriptor
            .aliases
            .iter()
            .any(|alias| is_reserved_builtin_alias(alias))
    {
        return Err(format!(
            "external descriptor `{}` collides with a reserved builtin alias",
            identity.descriptor_id
        ));
    }
    Ok(())
}

pub(super) fn is_reserved_builtin_alias(alias: &str) -> bool {
    matches!(
        alias.trim(),
        "Run"
            | "Ask"
            | "Read"
            | "List"
            | "Search"
            | "Glob"
            | "WebFetch"
            | "WebSearch"
            | "MCPResource"
            | "ToolSearch"
            | "Write"
            | "Edit"
            | "BatchExecute"
            | "Plan"
            | "ViewImage"
    ) || alias.starts_with("workspace.")
        || alias.starts_with("workspace_")
        || alias.starts_with("time.")
        || alias.starts_with("echo.")
        || alias.starts_with("web.")
        || alias.starts_with("tool.")
        || matches!(
            alias.trim(),
            "time_now"
                | "echo_input"
                // 小写 `ask` 一并保留：外部 MCP/skill 注册名为 `ask` 的工具会被前端
                // isAskToolName 误标为 Ask 卡片，故从注册源头拒绝（PA-114 评审裁决）。
                | "ask"
                | "ask_user"
                | "ask.user"
                | "web_fetch_url"
                | "web_search_query"
                | "mcp_resource_read"
                | "mcp.resource_read"
                | "tool_search"
                | "plan_control"
                | "plan.control"
                | "view_image"
                | "view.image"
        )
}

pub(super) fn validate_composite_dependencies(
    descriptors: &[ToolDescriptor],
    descriptor_ids: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let dependencies = descriptors
        .iter()
        .map(|descriptor| {
            (
                descriptor.identity.descriptor_id.as_str(),
                descriptor.composed_descriptor_ids.as_slice(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    for descriptor in descriptors {
        for child_id in &descriptor.composed_descriptor_ids {
            if !descriptor_ids.contains(child_id) {
                return Err(format!(
                    "tool registry descriptor `{}` references unknown child `{child_id}`",
                    descriptor.identity.descriptor_id
                ));
            }
        }
    }

    fn visit(
        descriptor_id: &str,
        dependencies: &std::collections::BTreeMap<&str, &[String]>,
        visiting: &mut std::collections::BTreeSet<String>,
        visited: &mut std::collections::BTreeSet<String>,
    ) -> Result<(), String> {
        if visited.contains(descriptor_id) {
            return Ok(());
        }
        if !visiting.insert(descriptor_id.to_string()) {
            return Err(format!(
                "tool registry composite dependency cycle includes `{descriptor_id}`"
            ));
        }
        for child in dependencies.get(descriptor_id).copied().unwrap_or_default() {
            visit(child, dependencies, visiting, visited)?;
        }
        visiting.remove(descriptor_id);
        visited.insert(descriptor_id.to_string());
        Ok(())
    }

    let mut visiting = std::collections::BTreeSet::new();
    let mut visited = std::collections::BTreeSet::new();
    for descriptor_id in dependencies.keys() {
        visit(descriptor_id, &dependencies, &mut visiting, &mut visited)?;
    }
    Ok(())
}
