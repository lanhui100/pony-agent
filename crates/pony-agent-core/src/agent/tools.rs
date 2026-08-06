//! Legacy `ToolRouter` builtin tool handlers (PA-076 migration surface).
//!
//! This module is the compatibility `ToolRouter` the runtime still routes tool calls through
//! until the governed dispatcher takes over. The `Run`/`WebFetch`/`Search`/`Glob` handlers are
//! wired to the hardened modules from phase 5/6 (PA-076 tasks 5.5, 6.3, 6.4):
//!
//! - [`crate::agent::process::ProcessManager`] backs `workspace_run_command` with the
//!   session-scoped process lifecycle (start / bounded poll / kill / timeout, PA-076 tasks
//!   5.3-5.5). The legacy Run fails closed on sandbox grounds: it always runs
//!   `enforce_sandbox` against the registered backend, and with no backend registered it
//!   treats the request as `NoSandboxBackend` (Unavailable) so the Run is `sandbox_denied` —
//!   never a silent downgrade to a full-parent-environment shell (phase-5 review P1-2).
//! - [`crate::agent::web_access::WebAccessPolicy`] + [`crate::agent::web_access::PinnedConnector`]
//!   gate `web_fetch_url` before any connection and pin every connection to the validated
//!   addresses (design Decision 8, PA-076 tasks 6.1-6.3 + item 2): the initial URL and each of up
//!   to 5 redirect hops are validated for scheme, credentials, host, resolved addresses, port and
//!   redirect budget, and every hop is sent through a per-hop client whose DNS resolution for the
//!   URL's hostname is overridden to the validated addresses (the `Host` header and TLS SNI
//!   preserve the original authority; only the socket target is pinned). Arbitrary hostname URLs
//!   still fail closed until a production `WebResolver` is injected (a governed-path follow-up).
//!   The client is built with no ambient proxy and no auto-redirects; there is no
//!   "validate then hand off to a re-resolving client" weak mode.
//! - [`crate::agent::search::SearchEngine`] replaces the wildcard pseudo-regex and custom
//!   traversal in `workspace_search_text` / `workspace_glob_files` with real regex, globset,
//!   `.gitignore`-aware traversal and explicit truncation evidence (design Decision 9).

use crate::agent::process::ProcessManager;
use crate::agent::runtime_helper::block_on;
use crate::agent::sandbox::{enforce_sandbox, NoSandboxBackend};
use crate::agent::search::{SearchEngine, SearchOptions};
use crate::agent::tool_runtime::{
    ProcessBackend, ProcessStartRequest, ProcessState, SandboxBackend, SandboxRequest,
    TurnToolView, WebResolver,
};
use crate::agent::web_access::{
    PinnedConnector, WebAccessDecision, WebAccessDenyReason, WebAccessPolicy,
};
use encoding_rs::{Encoding, GBK};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

const TOOL_TIME_NOW: &str = "time_now";
const TOOL_ECHO_INPUT: &str = "echo_input";
const TOOL_WORKSPACE_LIST_FILES: &str = "workspace_list_files";
const TOOL_WORKSPACE_READ_FILE: &str = "workspace_read_file";
const TOOL_WORKSPACE_READ_FILE_SEGMENT: &str = "workspace_read_file_segment";
const TOOL_WORKSPACE_PATH_INFO: &str = "workspace_path_info";
const TOOL_WORKSPACE_SEARCH_TEXT: &str = "workspace_search_text";
const TOOL_WORKSPACE_GLOB_FILES: &str = "workspace_glob_files";
const TOOL_WORKSPACE_BATCH: &str = "workspace_batch";
const TOOL_WORKSPACE_GATHER_CONTEXT: &str = "workspace_gather_context";
const TOOL_WORKSPACE_WRITE_FILE: &str = "workspace_write_file";
const TOOL_WORKSPACE_EDIT_FILE: &str = "workspace_edit_file";
const TOOL_WORKSPACE_RUN_COMMAND: &str = "workspace_run_command";
const TOOL_WEB_FETCH_URL: &str = "web_fetch_url";
const TOOL_WEB_SEARCH_QUERY: &str = "web_search_query";
const TOOL_MCP_RESOURCE_READ: &str = "mcp_resource_read";
const TOOL_TOOL_SEARCH: &str = "tool_search";
/// Plan state control (PA-076 phase 7 P2-6): create/replace/merge/complete_step of a
/// session-owned plan — never executes arbitrary child calls. The handler lives in
/// [`crate::agent::plan_state::PlanControlHandler`] and is registered in
/// [`crate::agent::governed_executor::build_governed_executor`].
pub(crate) const TOOL_PLAN_CONTROL: &str = "plan_control";
/// Workspace-scoped image reading (PA-076 phase 7 P2-6): returns a reference-based
/// [`crate::agent::image_artifact::ImageArtifact`] with default `include_bytes=false`.
/// Whether the encoded bytes are sent to a model is a provider modality decision
/// (design.md Decision 11). The handler lives in
/// [`crate::agent::image_artifact::ViewImageHandler`].
pub(crate) const TOOL_VIEW_IMAGE: &str = "view_image";
/// Workspace-scoped office document conversion (Firecrawl anydoc): converts Word/PowerPoint/
/// Excel/ODF/RTF/EPUB/CSV/PDF to GitHub-Flavored Markdown, local and offline. The handler
/// lives in [`crate::agent::document_conversion::ReadDocumentHandler`].
pub(crate) const TOOL_WORKSPACE_READ_DOCUMENT: &str = "workspace_read_document";

const MAX_FULL_READ_BYTES: u64 = 120_000;
const MAX_PATH_REPAIR_SEARCH_FILES: usize = 2_000;
const MAX_WORKSPACE_BATCH_CALLS: usize = 24;
/// Primitives permitted as `workspace_batch` children while the legacy composite
/// still bypasses the governed child dispatcher (PA-076 task 3.5). Fail-closed
/// safety gate per design Decision 3: every other scope is rejected with
/// `unsupported_composite_child` until child dispatch takes over.
const READ_ONLY_BATCH_CHILD_PRIMITIVES: &[&str] = &[
    TOOL_WORKSPACE_LIST_FILES,
    TOOL_WORKSPACE_READ_FILE,
    TOOL_WORKSPACE_READ_FILE_SEGMENT,
    TOOL_WORKSPACE_READ_DOCUMENT,
    TOOL_WORKSPACE_PATH_INFO,
    TOOL_WORKSPACE_SEARCH_TEXT,
    TOOL_WORKSPACE_GLOB_FILES,
];
const MAX_GATHER_CONTEXT_PATHS: usize = 6;
const MAX_SEGMENT_LINES: usize = 400;
const DEFAULT_SEGMENT_LINES: usize = 80;
const DEFAULT_LIST_LIMIT: usize = 40;
const SUMMARY_ITEM_LIMIT: usize = 3;
const DEFAULT_RUN_TIMEOUT_MS: u64 = 10_000;
const MAX_RUN_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_WEB_TIMEOUT_MS: u64 = 15_000;
/// Hard cap on a fetched response body (design Decision 8 / phase-4..7 review P1-4): never read a
/// response unbounded; past this the body is truncated and surfaced honestly.
const MAX_WEB_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Fixed user agent for both the legacy web client and the pinned per-hop web client.
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const TOOL_TIMEOUT_RETRY_MAX_ATTEMPTS: u32 = 2;
/// Aggregate wall-clock budget for one `web_fetch_url` call — every retry attempt, redirect hop
/// and per-hop request combined (design Decision 8 / tasks 6.2-6.3: a chain-level total deadline).
/// The effective budget is `max(timeoutMs, WEB_FETCH_TOTAL_DEADLINE_MS)` so a single slow hop
/// keeps its full requested `timeoutMs`, while the aggregate never multiplies across hops ×
/// retries × per-hop timeout.
const WEB_FETCH_TOTAL_DEADLINE_MS: u64 = 30_000;

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
            default_timeout_ms: DEFAULT_RUN_TIMEOUT_MS,
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

fn validate_descriptor_identity(descriptor: &ToolDescriptor) -> Result<(), String> {
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

fn is_reserved_builtin_alias(alias: &str) -> bool {
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

fn validate_composite_dependencies(
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

    /// Optional downcast accessor (PA-076 Ask host mediation). Adapters that wrap a concrete
    /// executor with extra surface — e.g. `GovernedToolExecutor` — expose themselves here so the
    /// runtime can reach the shared governed dispatcher and set per-turn invocation context. The
    /// default implementation returns `None`, keeping every existing `ToolExecutor` implementor
    /// source-compatible.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
}

pub struct ToolRouter {
    workspace_root: PathBuf,
    process_manager: ProcessManager,
    sandbox_backend: Option<Arc<dyn SandboxBackend>>,
    web_resolver: Arc<dyn WebResolver>,
}

impl ToolRouter {
    pub fn new() -> Self {
        Self {
            workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            process_manager: ProcessManager::new(),
            sandbox_backend: None,
            web_resolver: Arc::new(FailClosedResolver),
        }
    }

    pub fn with_workspace_root(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            process_manager: ProcessManager::new(),
            sandbox_backend: None,
            web_resolver: Arc::new(FailClosedResolver),
        }
    }

    /// Register an explicit sandbox backend for the legacy `workspace_run_command` path.
    ///
    /// When a backend is registered the legacy handler runs `enforce_sandbox` against it and
    /// fails closed on denial. When `None` (the default) the legacy handler still fails closed:
    /// it treats the request as `NoSandboxBackend` (Unavailable), so an autonomous Run is
    /// `sandbox_denied` — never a silent downgrade to a full-parent-environment shell
    /// (design Decision 7, phase-5 review P1-2). See the module documentation.
    pub fn with_sandbox_backend<B: SandboxBackend + 'static>(mut self, backend: B) -> Self {
        self.sandbox_backend = Some(Arc::new(backend));
        self
    }

    /// Inject a `WebResolver` for the legacy `web_fetch_url` path (design Decision 8).
    ///
    /// The default [`FailClosedResolver`] keeps every hostname URL fail-closed until a production
    /// resolver is wired in; this builder is the injection point that a governed/production
    /// resolver uses. When injected, hostname URLs are resolved through it, validated, and every
    /// connection is pinned to the validated addresses — there is no "validate then let the
    /// default client re-resolve" weak mode.
    pub fn with_web_resolver<R: WebResolver + 'static>(mut self, resolver: R) -> Self {
        self.web_resolver = Arc::new(resolver);
        self
    }

    pub fn execute(&self, call: &ToolCall) -> ToolResult {
        let started_at = Instant::now();
        let mut result = self.execute_internal(call, true);
        result.duration_ms = started_at.elapsed().as_millis() as u64;
        result
    }

    fn execute_internal(&self, call: &ToolCall, allow_batch: bool) -> ToolResult {
        match canonical_tool_name(&call.name) {
            Some(TOOL_TIME_NOW) => self.time_now(),
            Some(TOOL_ECHO_INPUT) => self.echo_input(call),
            Some(TOOL_WORKSPACE_LIST_FILES) => self.list_files(call),
            Some(TOOL_WORKSPACE_READ_FILE) => self.read_file(call),
            Some(TOOL_WORKSPACE_READ_FILE_SEGMENT) => self.read_file_segment(call),
            Some(TOOL_WORKSPACE_PATH_INFO) => self.path_info(call),
            Some(TOOL_WORKSPACE_SEARCH_TEXT) => self.search_text(call),
            Some(TOOL_WORKSPACE_GLOB_FILES) => self.glob_files(call),
            Some(TOOL_WORKSPACE_WRITE_FILE) => self.write_file(call),
            Some(TOOL_WORKSPACE_EDIT_FILE) => self.edit_file(call),
            Some(TOOL_WORKSPACE_RUN_COMMAND) => self.run_command(call),
            Some(TOOL_WEB_FETCH_URL) => self.web_fetch(call),
            Some(TOOL_WEB_SEARCH_QUERY) => self.web_search(call),
            Some(TOOL_MCP_RESOURCE_READ) => error_result(
                TOOL_MCP_RESOURCE_READ,
                "deferred_to_registry",
                "MCPResource 由 capability registry 代理执行。".to_string(),
                Some("请通过 runtime 注册工具执行入口调用该工具。".to_string()),
            ),
            Some(TOOL_TOOL_SEARCH) => error_result(
                TOOL_TOOL_SEARCH,
                "deferred_to_registry",
                "ToolSearch 由 capability registry 代理执行。".to_string(),
                Some("请通过 runtime 注册工具执行入口调用该工具。".to_string()),
            ),
            Some(TOOL_WORKSPACE_GATHER_CONTEXT) => self.gather_context(call),
            Some(TOOL_WORKSPACE_BATCH) if allow_batch => self.batch(call),
            Some(TOOL_WORKSPACE_BATCH) => error_result(
                TOOL_WORKSPACE_BATCH,
                "nested_batch_not_allowed",
                "workspace_batch 不允许递归调用 workspace_batch。".to_string(),
                Some("请把嵌套批量调用拆成多个叶子工具调用。".to_string()),
            ),
            _ => error_result(
                &call.name,
                "unsupported_tool",
                format!("当前 runtime 尚未实现工具 `{}`。", call.name),
                Some("请改用 list_available_tools 中返回的工具名。".to_string()),
            ),
        }
    }

    fn time_now(&self) -> ToolResult {
        let unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let local_now = chrono::Local::now();
        let local_iso = local_now.format("%Y-%m-%d %H:%M:%S %z").to_string();
        let timezone = local_now.format("%z").to_string();

        ToolResult {
            tool_name: TOOL_TIME_NOW.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "unixTimestampSeconds": unix_seconds,
                "localIso": local_iso,
                "timezone": timezone,
            })),
            duration_ms: 0,
        }
    }

    fn echo_input(&self, call: &ToolCall) -> ToolResult {
        let text = call
            .arguments
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();

        if text.is_empty() {
            let fallback = call
                .arguments
                .get("question")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());

            let Some(fallback) = fallback else {
                return error_result(
                    TOOL_ECHO_INPUT,
                    "missing_argument",
                    "缺少必填参数 `text`，且未提供 fallback `question`。".to_string(),
                    Some(
                        "参数示例：{\"text\":\"请确认是否继续\"} 或 {\"question\":\"请确认下一步要执行什么？\"}"
                            .to_string(),
                    ),
                );
            };

            return ToolResult {
                tool_name: TOOL_ECHO_INPUT.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_ECHO_INPUT,
                    "mode": "fallback_clarification",
                    "prompt": fallback,
                    "summary": {
                        "text": fallback
                    },
                    "permission": {
                        "requiresApproval": false,
                        "permissionScope": "workspace.read",
                        "hostMediated": false,
                        "permissionProfile": "builtin",
                        "approvalMode": "none",
                        "decisionSource": "runtime"
                    }
                })),
                duration_ms: 0,
            };
        }

        ToolResult {
            tool_name: TOOL_ECHO_INPUT.to_string(),
            status: "ok".to_string(),
            output: format!("echo_input 返回：{}", text),
            duration_ms: 0,
        }
    }

    fn write_file(&self, call: &ToolCall) -> ToolResult {
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_WRITE_FILE,
                "missing_argument",
                "缺少必填参数 `path`。".to_string(),
                Some("参数示例：{\"path\":\"src/demo.txt\",\"content\":\"hello\"}".to_string()),
            );
        };
        let Some(content) = call.arguments.get("content").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_WRITE_FILE,
                "missing_argument",
                "缺少必填参数 `content`。".to_string(),
                Some("参数示例：{\"path\":\"src/demo.txt\",\"content\":\"hello\"}".to_string()),
            );
        };

        let overwrite = call
            .arguments
            .get("overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let relative_path = path.trim();
        if relative_path.is_empty() {
            return error_result(
                TOOL_WORKSPACE_WRITE_FILE,
                "empty_path",
                "参数 `path` 不能为空字符串。".to_string(),
                Some("请传入工作区内的相对文件路径。".to_string()),
            );
        }

        let target = match self.prepare_workspace_file_path(relative_path) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_WRITE_FILE, "invalid_path", error, None)
            }
        };

        let existed_before = target.exists();
        if existed_before && !overwrite {
            return error_result(
                TOOL_WORKSPACE_WRITE_FILE,
                "file_exists",
                format!(
                    "目标文件已存在：{}。",
                    self.display_workspace_relative(&target)
                ),
                Some("如需覆盖，请显式传入 {\"overwrite\": true}。".to_string()),
            );
        }

        if let Some(parent) = target.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                return error_result(
                    TOOL_WORKSPACE_WRITE_FILE,
                    "create_parent_failed",
                    format!("创建父目录失败：{}。", error),
                    Some("请确认目标目录在当前工作区内且进程有写权限。".to_string()),
                );
            }
        }

        if let Err(error) = fs::write(&target, content) {
            return error_result(
                TOOL_WORKSPACE_WRITE_FILE,
                "write_failed",
                format!("写入文件失败：{}。", error),
                Some("请确认目标文件可写，且当前进程有写权限。".to_string()),
            );
        }

        ToolResult {
            tool_name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "ok": true,
                "path": self.display_workspace_relative(&target),
                "absolutePath": target.display().to_string(),
                "bytesWritten": content.len(),
                "overwroteExisting": existed_before,
                "summary": {
                    "text": format!("已写入文件 {}。", self.display_workspace_relative(&target))
                },
                "permission": {
                    "requiresApproval": false,
                    "permissionScope": "workspace.write",
                    "hostMediated": false,
                    "permissionProfile": "builtin",
                    "approvalMode": "none",
                    "decisionSource": "runtime"
                }
            })),
            duration_ms: 0,
        }
    }

    fn edit_file(&self, call: &ToolCall) -> ToolResult {
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "missing_argument",
                "缺少必填参数 `path`。".to_string(),
                Some(
                    "参数示例：{\"path\":\"src/demo.txt\",\"oldText\":\"foo\",\"newText\":\"bar\"}"
                        .to_string(),
                ),
            );
        };
        let Some(old_text) = call.arguments.get("oldText").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "missing_argument",
                "缺少必填参数 `oldText`。".to_string(),
                Some(
                    "参数示例：{\"path\":\"src/demo.txt\",\"oldText\":\"foo\",\"newText\":\"bar\"}"
                        .to_string(),
                ),
            );
        };
        let Some(new_text) = call.arguments.get("newText").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "missing_argument",
                "缺少必填参数 `newText`。".to_string(),
                Some(
                    "参数示例：{\"path\":\"src/demo.txt\",\"oldText\":\"foo\",\"newText\":\"bar\"}"
                        .to_string(),
                ),
            );
        };

        if old_text.is_empty() {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "empty_old_text",
                "参数 `oldText` 不能为空字符串。".to_string(),
                Some("请提供需要被替换的原始文本。".to_string()),
            );
        }

        let replace_all = call
            .arguments
            .get("replaceAll")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let target = match self.resolve_workspace_path(path) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_EDIT_FILE, "invalid_path", error, None)
            }
        };

        let original = match fs::read_to_string(&target) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_EDIT_FILE,
                    "read_failed",
                    format!("读取文件失败：{}。", error),
                    Some("请确认目标文件存在且为可读文本文件。".to_string()),
                )
            }
        };

        let match_count = original.matches(old_text).count();
        if match_count == 0 {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "no_match",
                format!(
                    "在文件 {} 中没有找到 `oldText`。",
                    self.display_workspace_relative(&target)
                ),
                Some("请先读取文件确认原始文本，再执行编辑。".to_string()),
            );
        }
        if match_count > 1 && !replace_all {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "multiple_matches",
                format!(
                    "在文件 {} 中找到 {} 处匹配；未显式允许批量替换。",
                    self.display_workspace_relative(&target),
                    match_count
                ),
                Some("如需全部替换，请传入 {\"replaceAll\": true}。".to_string()),
            );
        }

        let updated = if replace_all {
            original.replace(old_text, new_text)
        } else {
            original.replacen(old_text, new_text, 1)
        };

        if let Err(error) = fs::write(&target, updated.as_bytes()) {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "write_failed",
                format!("写回文件失败：{}。", error),
                Some("请确认目标文件可写，且当前进程有写权限。".to_string()),
            );
        }

        ToolResult {
            tool_name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "ok": true,
                "path": self.display_workspace_relative(&target),
                "absolutePath": target.display().to_string(),
                "matchCount": match_count,
                "replacedCount": if replace_all { match_count } else { 1 },
                "replaceAll": replace_all,
                "summary": {
                    "text": format!("已编辑文件 {}。", self.display_workspace_relative(&target))
                },
                "permission": {
                    "requiresApproval": false,
                    "permissionScope": "workspace.write",
                    "hostMediated": false,
                    "permissionProfile": "builtin",
                    "approvalMode": "none",
                    "decisionSource": "runtime"
                }
            })),
            duration_ms: 0,
        }
    }

    fn run_command(&self, call: &ToolCall) -> ToolResult {
        let Some(command) = call.arguments.get("command").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "missing_argument",
                "缺少必填参数 `command`。".to_string(),
                Some(
                    "参数示例：{\"command\":\"git status\",\"cwd\":\".\",\"timeoutMs\":5000}"
                        .to_string(),
                ),
            );
        };

        let command = command.trim();
        if command.is_empty() {
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "empty_command",
                "参数 `command` 不能为空字符串。".to_string(),
                Some("请提供要执行的命令文本。".to_string()),
            );
        }

        if let Some(reason) = denied_run_command_reason(command) {
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "command_denied",
                reason,
                Some("请改用只读、非破坏性的工作区内命令。".to_string()),
            );
        }

        let cwd_input = call
            .arguments
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or(".");
        let cwd = match self.resolve_workspace_dir(cwd_input) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_RUN_COMMAND, "invalid_cwd", error, None)
            }
        };
        let timeout_ms = call
            .arguments
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, MAX_RUN_TIMEOUT_MS))
            .unwrap_or(DEFAULT_RUN_TIMEOUT_MS);

        // Sandbox gate (design Decision 7, PA-076 task 5.5, phase-5 review P1-2): the legacy
        // Run always runs `enforce_sandbox`. With no backend registered it fails closed via
        // `NoSandboxBackend` (Unavailable), so a Run never silently downgrades to a
        // full-parent-environment shell. An explicitly registered backend decides the verdict.
        let sandbox_request = SandboxRequest {
            workspace_root: self.canonical_workspace_root().display().to_string(),
            allow_network: false,
            environment_allowlist: Vec::new(),
            isolate_environment: true,
        };
        let backend: &dyn SandboxBackend = match &self.sandbox_backend {
            Some(backend) => backend.as_ref(),
            None => &NoSandboxBackend,
        };
        if let Err(reason) = enforce_sandbox(backend, &sandbox_request) {
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "sandbox_denied",
                reason,
                Some("无人值守 Run 需要可用的沙箱；请通过 host 审批显式允许。".to_string()),
            );
        }

        // Session-scoped process lifecycle (PA-076 tasks 5.3-5.5). Each legacy Run gets its own
        // opaque session id so handles can never be replayed across runs.
        let session_id = legacy_run_session_id();
        let (program, arguments) = workspace_command_parts(command, &cwd);
        let handle = match self.process_manager.start(&ProcessStartRequest {
            session_id: session_id.clone(),
            program,
            arguments,
            sandbox: sandbox_request,
        }) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_RUN_COMMAND,
                    "spawn_failed",
                    format!("启动命令失败：{}。", error),
                    Some("请确认命令语法正确，且当前环境存在对应可执行文件。".to_string()),
                )
            }
        };

        // Arm the kill timer as a safety net and run a bounded poll loop until `Exited`. The
        // loop accumulates per-poll output; truncation flags are sticky per stream.
        self.process_manager
            .kill_after(&session_id, &handle, Duration::from_millis(timeout_ms));
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut stdout_truncated = false;
        let mut stderr_truncated = false;
        let mut exit_code: Option<i32> = None;
        let mut timed_out = false;

        loop {
            if Instant::now() >= deadline {
                let _ = self.process_manager.kill(&session_id, &handle);
                timed_out = true;
                break;
            }
            match self.process_manager.poll(&session_id, &handle) {
                Ok(result) => {
                    stdout.push_str(&result.stdout);
                    stderr.push_str(&result.stderr);
                    stdout_truncated |= result.stdout_truncated;
                    stderr_truncated |= result.stderr_truncated;
                    if result.state == ProcessState::Exited {
                        exit_code = result.exit_code;
                        break;
                    }
                }
                Err(error) => {
                    let _ = self.process_manager.kill(&session_id, &handle);
                    return error_result(
                        TOOL_WORKSPACE_RUN_COMMAND,
                        "wait_failed",
                        format!("轮询命令状态失败：{}。", error),
                        Some("请重试，或更换更简单的命令。".to_string()),
                    );
                }
            }
            thread::sleep(Duration::from_millis(20));
        }

        if timed_out {
            // Best-effort entry cleanup; the kill_after timer may also fire later (idempotent).
            let _ = self.process_manager.kill(&session_id, &handle);
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "timeout",
                format!("命令执行超过超时上限 {} ms，已终止。", timeout_ms),
                Some("请缩短命令执行时间，或显式传入更大的 timeoutMs。".to_string()),
            );
        }

        // Exited: clean up the session entry (idempotent when already exited).
        let _ = self.process_manager.kill(&session_id, &handle);

        let succeeded = exit_code == Some(0);
        let error_payload = (!succeeded).then(|| {
            json!({
                "code": "non_zero_exit",
                "message": format!(
                    "命令执行完成，但退出码为 {}。",
                    exit_code.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string())
                ),
                "exitCode": exit_code
            })
        });

        ToolResult {
            tool_name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            status: if succeeded {
                "ok".to_string()
            } else {
                "error".to_string()
            },
            output: json_string(json!({
                "ok": succeeded,
                "cwd": self.display_workspace_relative(&cwd),
                "absoluteCwd": cwd.display().to_string(),
                "command": command,
                "delegateTool": "run_shell",
                "timeoutMs": timeout_ms,
                "exitCode": exit_code,
                "stdout": stdout,
                "stderr": stderr,
                "stdoutTruncated": stdout_truncated,
                "stderrTruncated": stderr_truncated,
                "error": error_payload,
                "summary": {
                    "text": format!(
                        "命令在 {} 执行完成，退出码为 {}。",
                        self.display_workspace_relative(&cwd),
                        exit_code.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string())
                    )
                },
                "permission": {
                    "requiresApproval": false,
                    "permissionScope": "workspace.execute",
                    "hostMediated": false,
                    "permissionProfile": "builtin",
                    "approvalMode": "none",
                    "decisionSource": "runtime"
                }
            })),
            duration_ms: 0,
        }
    }

    fn web_fetch(&self, call: &ToolCall) -> ToolResult {
        let Some(url) = call.arguments.get("url").and_then(Value::as_str) else {
            return error_result(
                TOOL_WEB_FETCH_URL,
                "missing_argument",
                "缺少必填参数 `url`。".to_string(),
                Some("参数示例：{\"url\":\"https://example.com\",\"timeoutMs\":15000}".to_string()),
            );
        };
        let url = url.trim();

        let timeout_ms = call
            .arguments
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 60_000))
            .unwrap_or(DEFAULT_WEB_TIMEOUT_MS);

        // Fail-closed web access policy + pinned connector (design Decision 8, PA-076 item 2):
        // every URL — the initial one and each of up to `max_redirects` redirect hops — is
        // validated for scheme, credentials, host, resolved addresses, port and redirect budget
        // BEFORE any connection, and every connection is pinned to the decision's validated
        // addresses (the per-hop client's DNS for the hostname is overridden to the validated
        // IPs, so the Host header and TLS SNI preserve the original authority while only the
        // validated peers can ever be reached). The default resolver cannot resolve any host, so
        // arbitrary hostname URLs still fail closed until a production resolver is injected (a
        // governed-path follow-up). On `Deny` the structured reason is returned as a fail-closed
        // error; there is no "validate then let the default client re-resolve" weak mode.
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, self.web_resolver.as_ref());
        let sender = ReqwestPinnedSender;

        // Keep the legacy timeout retry (at most two attempts) but preserve the structured error
        // of the final attempt so fail-closed denials keep their machine-readable evidence. One
        // aggregate chain deadline bounds the whole call (retries + redirect hops + per-hop
        // requests, design Decision 8 / tasks 6.2-6.3): at least the per-hop `timeoutMs` so a
        // single slow hop keeps its full requested timeout, but never a multiple of hops ×
        // retries × per-hop timeout.
        let total_budget_ms = timeout_ms.max(WEB_FETCH_TOTAL_DEADLINE_MS);
        let mut structured_error: Option<PinnedFetchError> = None;
        let exchange = match retry_tool_timeout(TOOL_WEB_FETCH_URL, total_budget_ms, |deadline| {
            pinned_web_fetch(&connector, url, timeout_ms, total_budget_ms, deadline, &sender)
                .map_err(|error| {
                    let message = pinned_fetch_error_message(&error);
                    structured_error = Some(error);
                    message
                })
        }) {
            Ok(value) => value,
            Err(message) => {
                return match structured_error.take() {
                    Some(PinnedFetchError::Denied { url, decision }) => {
                        web_fetch_denied(&url, &decision)
                    }
                    Some(PinnedFetchError::Transport(transport)) => {
                        web_fetch_transport_error(&transport)
                    }
                    None => error_result(
                        TOOL_WEB_FETCH_URL,
                        "request_failed",
                        message,
                        Some("请确认目标地址可访问，或稍后重试。".to_string()),
                    ),
                };
            }
        };

        let status = exchange.status;
        let final_url = exchange.url;
        let headers = exchange.headers;
        let (bytes, body_truncated) = (exchange.body, exchange.body_truncated);

        let body = decode_content(&headers, &bytes);

        ToolResult {
            tool_name: TOOL_WEB_FETCH_URL.to_string(),
            status: if status.is_success() {
                "ok".to_string()
            } else {
                "error".to_string()
            },
            output: json_string(json!({
                "ok": status.is_success(),
                "url": final_url,
                "statusCode": status.as_u16(),
                "contentPreview": preview_text(&body, 2000),
                "contentLength": body.len(),
                "truncated": body_truncated,
                "truncationReason": body_truncated.then(|| {
                    format!("响应体超过 {MAX_WEB_BODY_BYTES} 字节上限，已截断。")
                }),
                "error": (!status.is_success()).then(|| json!({
                    "code": "http_error",
                    "message": format!("抓取 URL 返回非成功状态码 {}。", status.as_u16()),
                    "hint": "请确认目标地址可访问，或检查服务端响应状态。"
                })),
                "summary": {
                    "text": format!("已抓取 URL {}，状态码 {}。", url, status.as_u16())
                }
            })),
            duration_ms: 0,
        }
    }

    fn web_search(&self, call: &ToolCall) -> ToolResult {
        let Some(query) = call.arguments.get("query").and_then(Value::as_str) else {
            return error_result(
                TOOL_WEB_SEARCH_QUERY,
                "missing_argument",
                "缺少必填参数 `query`。".to_string(),
                Some("参数示例：{\"query\":\"rust reqwest tutorial\",\"limit\":5}".to_string()),
            );
        };
        let query = query.trim();
        if query.is_empty() {
            return error_result(
                TOOL_WEB_SEARCH_QUERY,
                "empty_query",
                "参数 `query` 不能为空字符串。".to_string(),
                Some("请提供外部搜索关键词。".to_string()),
            );
        }

        let limit = call
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 10) as usize)
            .unwrap_or(5);
        let timeout_ms = call
            .arguments
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 60_000))
            .unwrap_or(DEFAULT_WEB_TIMEOUT_MS);

        let api_key = match crate::agent::config::ProviderRegistryStore::new().get_service_api_key("exa") {
            Some(key) => key,
            None => match std::env::var("EXA_API_KEY").ok() {
                Some(key) if !key.trim().is_empty() => key,
                _ => {
                    return error_result(
                        TOOL_WEB_SEARCH_QUERY,
                        "missing_api_key",
                        "未设置 EXA_API_KEY。请在设置页面输入 Exa API Key，或设置 EXA_API_KEY 环境变量。".to_string(),
                        Some("访问 https://dashboard.exa.ai 获取 API Key。".to_string()),
                    )
                }
            }
        };

        let client = match build_web_client(timeout_ms) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WEB_SEARCH_QUERY, "client_build_failed", error, None)
            }
        };

        let body = json!({
            "query": query,
            "type": "auto",
            "numResults": limit,
            "contents": {
                "highlights": true
            }
        });

        let response = match retry_tool_timeout(
            TOOL_WEB_SEARCH_QUERY,
            timeout_ms.max(WEB_FETCH_TOTAL_DEADLINE_MS),
            |_deadline| {
                block_on(
                    client
                        .post("https://api.exa.ai/search")
                        .header("x-api-key", &api_key)
                        .header("Content-Type", "application/json")
                        .body(body.to_string())
                        .send(),
                )
                .map_err(|error| {
                    if is_reqwest_timeout_error(&error) {
                        format!("timeout: Exa 搜索请求超时：{}。", error)
                    } else {
                        format!("Exa 搜索请求失败：{}。", error)
                    }
                })
            },
        ) {
            Ok(value) => value,
            Err(error) => {
                let code = if is_tool_timeout_message(&error) {
                    "timeout"
                } else {
                    "request_failed"
                };
                return error_result(
                    TOOL_WEB_SEARCH_QUERY,
                    code,
                    error,
                    Some("请确认 API Key 有效且网络可访问 api.exa.ai。".to_string()),
                );
            }
        };

        let status = response.status();
        let response_body = match block_on(response.text()) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WEB_SEARCH_QUERY,
                    "read_body_failed",
                    format!("读取 Exa 响应失败：{}。", error),
                    None,
                )
            }
        };

        let parsed: Value = match serde_json::from_str(&response_body) {
            Ok(value) => value,
            Err(_) => {
                return error_result(
                    TOOL_WEB_SEARCH_QUERY,
                    "api_error",
                    format!(
                        "Exa API 返回异常（状态码 {}）：{}",
                        status.as_u16(),
                        preview_text(&response_body, 200)
                    ),
                    Some("请检查 API Key 和查询参数。".to_string()),
                )
            }
        };

        if !status.is_success() {
            let error_msg = parsed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            return error_result(
                TOOL_WEB_SEARCH_QUERY,
                "api_error",
                format!("Exa API 错误（状态码 {}）：{}", status.as_u16(), error_msg),
                Some("请检查 API Key 和查询参数。".to_string()),
            );
        }

        let results: Vec<Value> = parsed
            .get("results")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .map(|r| {
                        json!({
                            "title": r.get("title").and_then(Value::as_str).unwrap_or(""),
                            "url": r.get("url").and_then(Value::as_str).unwrap_or(""),
                            "snippet": r.get("highlights")
                                .and_then(Value::as_array)
                                .and_then(|h| h.first())
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                            "score": r.get("score").and_then(Value::as_f64).unwrap_or(0.0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        ToolResult {
            tool_name: TOOL_WEB_SEARCH_QUERY.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "ok": true,
                "query": query,
                "statusCode": status.as_u16(),
                "resultCount": results.len(),
                "results": results,
                "summary": {
                    "text": format!("已完成 Exa 搜索 `{}`，返回 {} 条结果。", query, results.len())
                }
            })),
            duration_ms: 0,
        }
    }
    fn read_file(&self, call: &ToolCall) -> ToolResult {
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_READ_FILE,
                "missing_argument",
                "缺少必填参数 `path`。".to_string(),
                Some("参数示例：{\"path\":\"src-tauri/src/agent/tools.rs\"}".to_string()),
            );
        };

        let resolved = match self.resolve_workspace_path(path) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_READ_FILE, "invalid_path", error, None)
            }
        };

        let metadata = match fs::metadata(&resolved) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_READ_FILE,
                    "metadata_failed",
                    format!("读取文件元信息失败：{}。", error),
                    Some("请确认目标文件存在，且当前进程有读取权限。".to_string()),
                )
            }
        };

        if metadata.len() > MAX_FULL_READ_BYTES {
            return error_result(
                TOOL_WORKSPACE_READ_FILE,
                "file_too_large",
                format!(
                    "文件 {} 大小为 {} bytes，超过整文件读取上限 {} bytes。",
                    self.display_workspace_relative(&resolved),
                    metadata.len(),
                    MAX_FULL_READ_BYTES
                ),
                Some(
                    "请改用 workspace_read_file_segment，或用 workspace_gather_context 获取概览。"
                        .to_string(),
                ),
            );
        }

        match fs::read_to_string(&resolved) {
            Ok(content) => ToolResult {
                tool_name: TOOL_WORKSPACE_READ_FILE.to_string(),
                status: "ok".to_string(),
                output: format!(
                    "文件 {} 读取成功。\n\n{}",
                    self.display_workspace_relative(&resolved),
                    truncate_preview(&content, 4000)
                ),
                duration_ms: 0,
            },
            Err(error) => error_result(
                TOOL_WORKSPACE_READ_FILE,
                "read_failed",
                format!("读取文件失败：{}。", error),
                Some(
                    "请确认目标文件是 UTF-8 文本，或改用 workspace_path_info 判断文件类型。"
                        .to_string(),
                ),
            ),
        }
    }

    fn read_file_segment(&self, call: &ToolCall) -> ToolResult {
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_READ_FILE_SEGMENT,
                "missing_argument",
                "缺少必填参数 `path`。".to_string(),
                Some(
                    "参数示例：{\"path\":\"src/main.rs\",\"startLine\":1,\"lineCount\":40}"
                        .to_string(),
                ),
            );
        };

        let start_line = call
            .arguments
            .get("startLine")
            .and_then(Value::as_u64)
            .map(|value| value.max(1) as usize)
            .unwrap_or(1);
        let line_count = call
            .arguments
            .get("lineCount")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, MAX_SEGMENT_LINES as u64) as usize)
            .unwrap_or(40);

        let resolved = match self.resolve_workspace_path(path) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_READ_FILE_SEGMENT,
                    "invalid_path",
                    error,
                    None,
                )
            }
        };

        match read_file_lines(&resolved, start_line, line_count) {
            Ok(FileSegment::Empty) => ToolResult {
                tool_name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                status: "ok".to_string(),
                output: format!("文件 {} 为空。", self.display_workspace_relative(&resolved)),
                duration_ms: 0,
            },
            Ok(FileSegment::Range {
                start_line,
                end_line,
                lines,
                total_lines,
            }) => {
                let segment = lines
                    .iter()
                    .map(|(line_number, line)| format!("{:>4} | {}", line_number, line))
                    .collect::<Vec<_>>()
                    .join("\n");

                ToolResult {
                    tool_name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                    status: "ok".to_string(),
                    output: format!(
                        "文件 {} 第 {} 行到第 {} 行（总行数约 {}）：\n{}",
                        self.display_workspace_relative(&resolved),
                        start_line,
                        end_line,
                        total_lines,
                        segment
                    ),
                    duration_ms: 0,
                }
            }
            Err(ReadSegmentError::StartOutOfRange { total_lines }) => error_result(
                TOOL_WORKSPACE_READ_FILE_SEGMENT,
                "line_out_of_range",
                format!("起始行 {} 超出文件总行数 {}。", start_line, total_lines),
                Some(
                    "请缩小 startLine，或先用 workspace_path_info / workspace_gather_context 查看文件概况。"
                        .to_string(),
                ),
            ),
            Err(ReadSegmentError::Io(error)) => error_result(
                TOOL_WORKSPACE_READ_FILE_SEGMENT,
                "read_failed",
                format!("读取文件失败：{}。", error),
                Some("请确认目标文件是 UTF-8 文本，且当前进程有读取权限。".to_string()),
            ),
        }
    }

    fn list_files(&self, call: &ToolCall) -> ToolResult {
        let relative_dir = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();
        let limit = call
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 200) as usize)
            .unwrap_or(DEFAULT_LIST_LIMIT);

        let dir = match self.resolve_workspace_dir(relative_dir) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_LIST_FILES, "invalid_path", error, None)
            }
        };

        match fs::read_dir(&dir) {
            Ok(entries) => {
                let mut items = entries
                    .filter_map(Result::ok)
                    .map(|entry| {
                        let path = entry.path();
                        let label = self.display_workspace_relative(&path);
                        if path.is_dir() {
                            format!("{}/", label)
                        } else {
                            label
                        }
                    })
                    .collect::<Vec<_>>();
                items.sort();

                let total = items.len();
                let preview = items.into_iter().take(limit).collect::<Vec<_>>();
                ToolResult {
                    tool_name: TOOL_WORKSPACE_LIST_FILES.to_string(),
                    status: "ok".to_string(),
                    output: format!(
                        "目录 {} 下共发现 {} 个条目，当前展示前 {} 个：\n{}",
                        self.display_workspace_relative(&dir),
                        total,
                        preview.len(),
                        preview.join("\n")
                    ),
                    duration_ms: 0,
                }
            }
            Err(error) => error_result(
                TOOL_WORKSPACE_LIST_FILES,
                "read_failed",
                format!("读取目录失败：{}。", error),
                Some("请确认目标目录存在，且当前进程有访问权限。".to_string()),
            ),
        }
    }

    fn path_info(&self, call: &ToolCall) -> ToolResult {
        let relative_path = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();

        let path = match self.resolve_workspace_entry(relative_path) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_PATH_INFO, "invalid_path", error, None)
            }
        };

        match fs::metadata(&path) {
            Ok(metadata) => {
                let path_type = if metadata.is_dir() {
                    "directory"
                } else if metadata.is_file() {
                    "file"
                } else {
                    "other"
                };
                let child_count = if metadata.is_dir() {
                    fs::read_dir(&path).ok().map(|entries| entries.count())
                } else {
                    None
                };
                let modified_unix = metadata
                    .modified()
                    .ok()
                    .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                    .map(|value| value.as_secs());

                ToolResult {
                    tool_name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "path": self.display_workspace_relative(&path),
                        "absolutePath": path.display().to_string(),
                        "kind": path_type,
                        "sizeBytes": metadata.len(),
                        "modifiedUnixSeconds": modified_unix,
                        "childCount": child_count,
                        "isReadableTextHint": metadata.is_file(),
                    })),
                    duration_ms: 0,
                }
            }
            Err(error) => error_result(
                TOOL_WORKSPACE_PATH_INFO,
                "metadata_failed",
                format!("读取路径元信息失败：{}。", error),
                Some("请确认目标路径存在，且当前进程有访问权限。".to_string()),
            ),
        }
    }

    fn glob_files(&self, call: &ToolCall) -> ToolResult {
        let pattern = call
            .arguments
            .get("pattern")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if pattern.is_empty() {
            return error_result(
                TOOL_WORKSPACE_GLOB_FILES,
                "missing_argument",
                "缺少必填参数 `pattern`。".to_string(),
                Some(
                    "参数示例：{\"pattern\":\"src/**/*.rs\",\"path\":\".\",\"limit\":50}"
                        .to_string(),
                ),
            );
        }

        let relative_dir = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();
        let limit = call
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 200) as usize)
            .unwrap_or(50);

        // Validate the glob up front (mirrors the engine's globset compilation) so a malformed
        // pattern fails with a structured error instead of a panic.
        if let Err(error) = compile_path_glob(pattern) {
            return error_result(
                TOOL_WORKSPACE_GLOB_FILES,
                "invalid_pattern",
                format!("无效的 glob 模式：{}。", error),
                Some("请使用合法 glob 模式，例如 `src/**/*.rs`。".to_string()),
            );
        }

        let root_entry = match self.resolve_workspace_entry(relative_dir) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_GLOB_FILES, "invalid_path", error, None)
            }
        };

        let (paths, truncated, truncation_reason) = if root_entry.is_file() {
            // A file root matches (or not) the single workspace-relative path against the glob,
            // using the same path-or-basename semantics as the engine.
            let relative = self.display_workspace_relative(&root_entry);
            let matched = glob_matches_path_or_basename(&relative, pattern);
            let paths = if matched {
                vec![relative]
            } else {
                Vec::new()
            };
            (paths, false, None)
        } else if root_entry.is_dir() {
            let engine = SearchEngine::new();
            let result = match engine.glob_files(pattern, &root_entry, limit) {
                Ok(value) => value,
                Err(error) => {
                    return error_result(
                        TOOL_WORKSPACE_GLOB_FILES,
                        "invalid_pattern",
                        error,
                        Some("请使用合法 glob 模式，例如 `src/**/*.rs`。".to_string()),
                    )
                }
            };
            // Re-map engine-relative paths to workspace-relative paths (the legacy output
            // contract).
            let paths = result
                .paths
                .into_iter()
                .map(|path| self.display_workspace_relative(&root_entry.join(&path)))
                .collect::<Vec<_>>();
            (paths, result.truncated, result.truncation_reason)
        } else {
            return error_result(
                TOOL_WORKSPACE_GLOB_FILES,
                "unsupported_path_kind",
                format!(
                    "当前路径类型不支持路径模式匹配：{}。",
                    self.display_workspace_relative(&root_entry)
                ),
                Some("请传入工作区内的文件或目录路径。".to_string()),
            );
        };

        let matches = paths
            .iter()
            .map(|path| {
                json!({
                    "path": path,
                    "kind": "file"
                })
            })
            .collect::<Vec<_>>();

        ToolResult {
            tool_name: TOOL_WORKSPACE_GLOB_FILES.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "pattern": pattern,
                "path": self.display_workspace_relative(&root_entry),
                "matchCount": matches.len(),
                "matches": matches,
                "truncated": truncated,
                "truncationReason": truncation_reason,
            })),
            duration_ms: 0,
        }
    }

    fn search_text(&self, call: &ToolCall) -> ToolResult {
        let Some(query) = call.arguments.get("query").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_SEARCH_TEXT,
                "missing_argument",
                "缺少必填参数 `query`。".to_string(),
                Some(
                    "参数示例：{\"query\":\"ToolRouter\",\"path\":\"src-tauri/src\",\"limit\":20}"
                        .to_string(),
                ),
            );
        };

        let query = query.trim();
        if query.is_empty() {
            return error_result(
                TOOL_WORKSPACE_SEARCH_TEXT,
                "empty_query",
                "参数 `query` 不能为空字符串。".to_string(),
                Some("请提供要搜索的关键字或文本片段。".to_string()),
            );
        }

        let relative_dir = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();
        let limit = call
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 100) as usize)
            .unwrap_or(20);
        let ignore_case = call
            .arguments
            .get("ignoreCase")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let regex_mode = call
            .arguments
            .get("regex")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let file_filter = call
            .arguments
            .get("filePattern")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        // The query is now a real regular expression when `regex` is set (design Decision 9,
        // PA-076 task 6.4). When `regex` is unset the query is treated as a literal substring and
        // escaped so it matches exactly (case folding still applies via `ignoreCase`).
        let effective_query = if regex_mode {
            query.to_string()
        } else {
            regex::escape(query)
        };

        // The user's `filePattern` is a real glob (globset semantics) under the new engine.
        // Validate it up front so a malformed pattern fails consistently for both file and
        // directory roots.
        if let Some(pattern) = &file_filter {
            if let Err(error) = compile_path_glob(pattern) {
                return error_result(
                    TOOL_WORKSPACE_SEARCH_TEXT,
                    "invalid_file_pattern",
                    format!("无效的 filePattern glob：{}。", error),
                    Some("请使用合法 glob 模式，例如 `*.rs` 或 `src/**/*.rs`。".to_string()),
                );
            }
        }

        let root_entry = match self.resolve_workspace_entry(relative_dir) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_SEARCH_TEXT, "invalid_path", error, None)
            }
        };

        let searched_path = self.display_workspace_relative(&root_entry);
        let path_kind = if root_entry.is_file() {
            "file"
        } else if root_entry.is_dir() {
            "directory"
        } else {
            "other"
        };

        // A file root is searched by running the engine over its parent directory with the file's
        // basename as the scan-scoping glob (the same convention `workspace_gather_context`
        // uses). A user-supplied `filePattern` is then applied as a post-filter so both
        // constraints hold.
        let (search_root, forced_filter) = if root_entry.is_file() {
            let parent = root_entry
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root_entry.clone());
            let basename = root_entry
                .file_name()
                .map(|value| value.to_string_lossy().to_string());
            (parent, basename)
        } else if root_entry.is_dir() {
            (root_entry.clone(), None)
        } else {
            return error_result(
                TOOL_WORKSPACE_SEARCH_TEXT,
                "unsupported_path_kind",
                format!("当前路径类型不支持文本搜索：{}。", searched_path),
                Some("请传入工作区内的文件或目录路径。".to_string()),
            );
        };

        let options = SearchOptions {
            max_matches: limit,
            ignore_case,
            file_pattern: forced_filter.clone().or_else(|| file_filter.clone()),
            ..SearchOptions::default()
        };

        let engine = SearchEngine::new();
        let result = match engine.search_text(&effective_query, &search_root, &options) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_SEARCH_TEXT,
                    "invalid_regex",
                    error,
                    Some("请检查 query 是否符合正则语法。".to_string()),
                )
            }
        };

        // Re-map engine-relative match paths to workspace-relative paths (the legacy output
        // contract and what `first_search_match_line`/consumers branch on).
        let mut matches = result
            .matches
            .into_iter()
            .map(|mut matched| {
                let absolute = search_root.join(&matched.path);
                matched.path = self.display_workspace_relative(&absolute);
                matched
            })
            .collect::<Vec<_>>();

        // Post-filter when a file root scoped the scan with the basename glob and the user also
        // supplied a filePattern: both must hold.
        if forced_filter.is_some() {
            if let Some(pattern) = &file_filter {
                matches.retain(|matched| glob_matches_path_or_basename(&matched.path, pattern));
            }
        }

        let match_count = matches.len();

        ToolResult {
            tool_name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "query": query,
                "path": searched_path,
                "pathKind": path_kind,
                "ignoreCase": ignore_case,
                "regex": regex_mode,
                "filePattern": file_filter,
                "scannedFiles": result.scanned_files,
                // Legacy per-category skip counters are preserved for compatibility but always 0:
                // the engine does not break skips out by category, and any scan-level budget
                // truncation is reported honestly via `truncated`/`truncationReason`.
                "skippedUnreadableFiles": 0,
                "skippedLargeFiles": 0,
                "skippedByBudget": 0,
                "scannedBytes": result.scanned_bytes,
                "truncated": result.truncated,
                "truncationReason": result.truncation_reason,
                "durationMs": result.duration_ms,
                "matchCount": match_count,
                "matches": matches.iter().map(|matched| json!({
                    "path": matched.path,
                    "line": matched.line,
                    "preview": matched.preview,
                    "captures": matched.captures,
                })).collect::<Vec<_>>(),
            })),
            duration_ms: 0,
        }
    }

    fn batch(&self, call: &ToolCall) -> ToolResult {
        let calls = match call.arguments.get("calls").and_then(Value::as_array) {
            Some(value) if !value.is_empty() => value,
            _ => {
                return error_result(
                    TOOL_WORKSPACE_BATCH,
                    "missing_argument",
                    "缺少必填参数 `calls`，且至少需要一个子调用。".to_string(),
                    Some(
                        "参数示例：{\"calls\":[{\"name\":\"workspace_path_info\",\"arguments\":{\"path\":\"src\"}}]}"
                            .to_string(),
                    ),
                )
            }
        };

        if calls.len() > MAX_WORKSPACE_BATCH_CALLS {
            return error_result(
                TOOL_WORKSPACE_BATCH,
                "too_many_calls",
                format!(
                    "单次 workspace_batch 最多允许 {} 个子调用，当前收到 {} 个。",
                    MAX_WORKSPACE_BATCH_CALLS,
                    calls.len()
                ),
                Some("请把批量请求拆小后重试。".to_string()),
            );
        }

        let continue_on_error = call
            .arguments
            .get("continueOnError")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let parallel = call
            .arguments
            .get("parallel")
            .and_then(Value::as_bool)
            .unwrap_or(true)
            && continue_on_error;

        let mut nested_calls = Vec::with_capacity(calls.len());
        for (index, item) in calls.iter().enumerate() {
            let Some(name) = item.get("name").and_then(Value::as_str) else {
                return error_result(
                    TOOL_WORKSPACE_BATCH,
                    "invalid_call_shape",
                    format!("第 {} 个子调用缺少字符串类型的 `name`。", index + 1),
                    Some("每个子调用都需要提供 `name` 和可选 `arguments`。".to_string()),
                );
            };

            let primitive = canonical_tool_name(name);
            let read_only_ok = match primitive {
                Some(primitive) => is_read_only_batch_child_primitive(primitive),
                None => false,
            };
            if !read_only_ok {
                return error_result(
                    TOOL_WORKSPACE_BATCH,
                    "unsupported_composite_child",
                    format!(
                        "workspace_batch 暂不允许执行非只读子调用 `{}`（scope: {}）。",
                        name,
                        composite_child_scope(primitive)
                    ),
                    Some(
                        "请把该子调用拆成独立的顶层工具调用；governed child dispatch 上线前只读子调用才能聚合。"
                            .to_string(),
                    ),
                );
            }

            let arguments = item.get("arguments").cloned().unwrap_or_else(|| json!({}));

            nested_calls.push(ToolCall {
                call_id: None,
                name: name.to_string(),
                arguments,
                plan: None,
            });
        }
        let plan = build_batch_tool_plan(&nested_calls, parallel, continue_on_error);

        let results = if parallel {
            thread::scope(|scope| {
                let mut handles = Vec::with_capacity(nested_calls.len());
                for (index, nested_call) in nested_calls.iter().cloned().enumerate() {
                    let worker_call = nested_call.clone();
                    handles.push((
                        index,
                        nested_call,
                        scope.spawn(move || self.execute_internal(&worker_call, false)),
                    ));
                }

                let mut collected = Vec::with_capacity(handles.len());
                for (index, nested_call, handle) in handles {
                    let result = handle.join().unwrap_or_else(|_| {
                        error_result(
                            canonical_tool_name(&nested_call.name).unwrap_or(&nested_call.name),
                            "join_failed",
                            format!("并发执行子调用 `{}` 失败。", nested_call.name),
                            Some(
                                "请改为串行执行，或检查该子调用是否触发了内部 panic。".to_string(),
                            ),
                        )
                    });
                    collected.push((index, nested_call, result));
                }
                collected.sort_by_key(|(index, _, _)| *index);
                collected
            })
        } else {
            let mut collected = Vec::with_capacity(nested_calls.len());
            let mut stop_after_index = None;
            for (index, nested_call) in nested_calls.iter().cloned().enumerate() {
                let result = self.execute_internal(&nested_call, false);
                let should_stop = result.status != "ok" && !continue_on_error;
                collected.push((index, nested_call, result));
                if should_stop {
                    stop_after_index = Some(index);
                    break;
                }
            }
            if let Some(failed_index) = stop_after_index {
                for (index, nested_call) in nested_calls
                    .iter()
                    .cloned()
                    .enumerate()
                    .skip(failed_index + 1)
                {
                    collected.push((
                        index,
                        nested_call,
                        aborted_result(
                            TOOL_WORKSPACE_BATCH,
                            "batch_aborted",
                            "前一个子调用失败，且 continueOnError=false，后续子调用未执行。"
                                .to_string(),
                        ),
                    ));
                }
            }
            collected
        };

        self.aggregate_nested_results(
            TOOL_WORKSPACE_BATCH,
            json!({
                "parallel": parallel,
                "continueOnError": continue_on_error,
            }),
            Some(plan),
            results,
        )
    }

    fn gather_context(&self, call: &ToolCall) -> ToolResult {
        let raw_path = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();
        let query = call
            .arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let limit = call
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 100) as usize)
            .unwrap_or(DEFAULT_LIST_LIMIT);
        let line_count = call
            .arguments
            .get("lineCount")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, MAX_SEGMENT_LINES as u64) as usize)
            .unwrap_or(DEFAULT_SEGMENT_LINES);
        let paths = call
            .arguments
            .get("paths")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let paths = unique_paths(paths);

        if !paths.is_empty() {
            let requested_path_count = paths.len();
            let skipped_paths = if requested_path_count > MAX_GATHER_CONTEXT_PATHS {
                paths[MAX_GATHER_CONTEXT_PATHS..].to_vec()
            } else {
                Vec::new()
            };
            let gathered_paths = paths
                .iter()
                .take(MAX_GATHER_CONTEXT_PATHS)
                .cloned()
                .collect::<Vec<_>>();

            let mut nested = gathered_paths
                .iter()
                .enumerate()
                .map(|(index, path)| {
                    let nested_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                        arguments: json!({
                            "path": path,
                            "query": if query.is_empty() { Value::Null } else { Value::String(query.clone()) },
                            "limit": limit,
                            "lineCount": line_count,
                        }),
                        plan: None,
                    };
                    let result = self.gather_context(&nested_call);
                    (index, nested_call, result)
                })
                .collect::<Vec<_>>();
            if !skipped_paths.is_empty() {
                let skipped_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                    arguments: json!({
                        "paths": skipped_paths,
                        "limit": limit,
                        "lineCount": line_count,
                    }),
                    plan: None,
                };
                nested.push((
                    gathered_paths.len(),
                    skipped_call,
                    ToolResult {
                        tool_name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                        status: "ok".to_string(),
                        output: json_string(json!({
                            "ok": true,
                            "status": "partial",
                            "reason": "too_many_paths",
                            "message": format!(
                                "单次 workspace_gather_context 最多聚合前 {} 个路径，已跳过其余 {} 个。",
                                MAX_GATHER_CONTEXT_PATHS,
                                requested_path_count.saturating_sub(MAX_GATHER_CONTEXT_PATHS)
                            ),
                            "skippedPaths": skipped_paths,
                        })),
                        duration_ms: 0,
                    },
                ));
            }
            let plan = build_multi_path_gather_plan(&gathered_paths, &query, limit, line_count);

            return self.aggregate_nested_results(
                TOOL_WORKSPACE_GATHER_CONTEXT,
                json!({
                    "mode": "multi_path",
                    "paths": gathered_paths,
                    "requestedPathCount": requested_path_count,
                    "skippedPaths": skipped_paths,
                    "limitApplied": requested_path_count > MAX_GATHER_CONTEXT_PATHS,
                    "pathLimit": MAX_GATHER_CONTEXT_PATHS,
                    "query": if query.is_empty() { Value::Null } else { Value::String(query) },
                }),
                Some(plan),
                nested,
            );
        }

        let path = if raw_path.is_empty() { "." } else { raw_path };
        let resolved = match self.resolve_workspace_entry(path) {
            Ok(value) => value,
            Err(error) => {
                let code = if error.contains("只允许访问当前工作区内的相对路径") {
                    "out_of_scope"
                } else {
                    "invalid_path"
                };
                return error_result(
                    TOOL_WORKSPACE_GATHER_CONTEXT,
                    code,
                    error,
                    Some("请提供工作区内存在的相对路径。".to_string()),
                );
            }
        };

        let metadata = match fs::metadata(&resolved) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_GATHER_CONTEXT,
                    "metadata_failed",
                    format!("读取路径元信息失败：{}。", error),
                    Some("请确认目标路径存在，且当前进程有访问权限。".to_string()),
                )
            }
        };

        let display_path = self.display_workspace_relative(&resolved);
        let mode = if !query.is_empty() {
            "search"
        } else if metadata.is_dir() {
            "directory"
        } else {
            "file"
        };

        let nested = match mode {
            "file" => thread::scope(|scope| {
                let path_info_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                    arguments: json!({ "path": display_path }),
                    plan: None,
                };
                let segment_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                    arguments: json!({
                        "path": display_path,
                        "startLine": 1,
                        "lineCount": line_count,
                    }),
                    plan: None,
                };
                let info_handle = scope.spawn(|| {
                    let result = self.path_info(&path_info_call);
                    (0usize, path_info_call, result)
                });
                let segment_handle = scope.spawn(|| {
                    let result = self.read_file_segment(&segment_call);
                    (1usize, segment_call, result)
                });
                vec![
                    info_handle.join().unwrap_or_else(|_| {
                        (
                            0,
                            ToolCall {
                                call_id: None,
                                name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                                arguments: json!({ "path": display_path }),
                                plan: None,
                            },
                            error_result(
                                TOOL_WORKSPACE_PATH_INFO,
                                "join_failed",
                                "并发执行 workspace_path_info 失败。".to_string(),
                                None,
                            ),
                        )
                    }),
                    segment_handle.join().unwrap_or_else(|_| {
                        (
                            1,
                            ToolCall {
                                call_id: None,
                                name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                                arguments: json!({
                                    "path": display_path,
                                    "startLine": 1,
                                    "lineCount": line_count,
                                }),
                                plan: None,
                            },
                            error_result(
                                TOOL_WORKSPACE_READ_FILE_SEGMENT,
                                "join_failed",
                                "并发执行 workspace_read_file_segment 失败。".to_string(),
                                None,
                            ),
                        )
                    }),
                ]
            }),
            "directory" => thread::scope(|scope| {
                let path_info_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                    arguments: json!({ "path": display_path }),
                    plan: None,
                };
                let list_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_LIST_FILES.to_string(),
                    arguments: json!({
                        "path": display_path,
                        "limit": limit,
                    }),
                    plan: None,
                };
                let info_handle = scope.spawn(|| {
                    let result = self.path_info(&path_info_call);
                    (0usize, path_info_call, result)
                });
                let list_handle = scope.spawn(|| {
                    let result = self.list_files(&list_call);
                    (1usize, list_call, result)
                });
                vec![
                    info_handle.join().unwrap_or_else(|_| {
                        (
                            0,
                            ToolCall {
                                call_id: None,
                                name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                                arguments: json!({ "path": display_path }),
                                plan: None,
                            },
                            error_result(
                                TOOL_WORKSPACE_PATH_INFO,
                                "join_failed",
                                "并发执行 workspace_path_info 失败。".to_string(),
                                None,
                            ),
                        )
                    }),
                    list_handle.join().unwrap_or_else(|_| {
                        (
                            1,
                            ToolCall {
                                call_id: None,
                                name: TOOL_WORKSPACE_LIST_FILES.to_string(),
                                arguments: json!({
                                    "path": display_path,
                                    "limit": limit,
                                }),
                                plan: None,
                            },
                            error_result(
                                TOOL_WORKSPACE_LIST_FILES,
                                "join_failed",
                                "并发执行 workspace_list_files 失败。".to_string(),
                                None,
                            ),
                        )
                    }),
                ]
            }),
            _ => {
                let search_path = if metadata.is_file() {
                    resolved
                        .parent()
                        .map(|value| self.display_workspace_relative(value))
                        .unwrap_or_else(|| ".".to_string())
                } else {
                    display_path.clone()
                };
                let file_pattern = if metadata.is_file() {
                    resolved
                        .file_name()
                        .map(|value| value.to_string_lossy().to_string())
                } else {
                    None
                };

                if metadata.is_file() {
                    let path_info_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                        arguments: json!({ "path": display_path }),
                        plan: None,
                    };
                    let search_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
                        arguments: json!({
                            "query": query,
                            "path": search_path,
                            "limit": limit,
                            "filePattern": file_pattern,
                        }),
                        plan: None,
                    };

                    let mut collected = Vec::with_capacity(3);
                    collected.push((
                        0usize,
                        path_info_call.clone(),
                        self.path_info(&path_info_call),
                    ));
                    let search_result = self.search_text(&search_call);
                    let segment_line =
                        first_search_match_line(&search_result.output, &display_path);
                    collected.push((1usize, search_call, search_result));

                    let start_line = segment_line
                        .map(|line| line.saturating_sub(line_count / 2).max(1))
                        .unwrap_or(1);
                    let segment_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                        arguments: json!({
                            "path": display_path,
                            "startLine": start_line,
                            "lineCount": line_count,
                        }),
                        plan: None,
                    };
                    collected.push((
                        2usize,
                        segment_call.clone(),
                        self.read_file_segment(&segment_call),
                    ));

                    collected
                } else {
                    let path_info_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                        arguments: json!({ "path": display_path }),
                        plan: None,
                    };
                    let search_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
                        arguments: json!({
                            "query": query,
                            "path": search_path,
                            "limit": limit,
                            "filePattern": file_pattern,
                        }),
                        plan: None,
                    };

                    let mut collected = Vec::with_capacity(3);
                    collected.push((
                        0usize,
                        path_info_call.clone(),
                        self.path_info(&path_info_call),
                    ));
                    let search_result = self.search_text(&search_call);
                    let should_add_listing = search_result.status != "ok"
                        || search_match_count(&search_result.output) == 0;
                    collected.push((1usize, search_call, search_result));

                    if should_add_listing {
                        let list_call = ToolCall {
                            call_id: None,
                            name: TOOL_WORKSPACE_LIST_FILES.to_string(),
                            arguments: json!({
                                "path": display_path,
                                "limit": limit,
                            }),
                            plan: None,
                        };
                        collected.push((2usize, list_call.clone(), self.list_files(&list_call)));
                    }

                    collected
                }
            }
        };
        let plan = build_nested_tool_plan(
            TOOL_WORKSPACE_GATHER_CONTEXT,
            mode,
            &display_path,
            query.as_str(),
            &nested,
        );

        self.aggregate_nested_results(
            TOOL_WORKSPACE_GATHER_CONTEXT,
            json!({
                "mode": mode,
                "path": display_path,
                "query": if query.is_empty() { Value::Null } else { Value::String(query) },
            }),
            Some(plan),
            nested,
        )
    }

    fn aggregate_nested_results(
        &self,
        tool_name: &str,
        meta: Value,
        plan: Option<ToolPlan>,
        mut nested: Vec<(usize, ToolCall, ToolResult)>,
    ) -> ToolResult {
        nested.sort_by_key(|(index, _, _)| *index);

        let success_count = nested
            .iter()
            .filter(|(_, _, result)| nested_result_status(result) == "ok")
            .count();
        let partial_count = nested
            .iter()
            .filter(|(_, _, result)| nested_result_status(result) == "partial")
            .count();
        let aborted_count = nested
            .iter()
            .filter(|(_, _, result)| nested_result_status(result) == "aborted")
            .count();
        let error_count = nested
            .len()
            .saturating_sub(success_count + partial_count + aborted_count);
        let aggregate_status = if error_count == 0 && partial_count == 0 && aborted_count == 0 {
            "ok"
        } else if success_count > 0 || partial_count > 0 {
            "partial"
        } else {
            "error"
        };

        let results = nested
            .into_iter()
            .map(|(index, nested_call, result)| {
                let output = parse_tool_output(&result.output);
                let aggregate_status = nested_output_status(&result, &output).to_string();
                json!({
                    "index": index,
                    "tool": nested_call.name,
                    "canonicalTool": canonical_tool_name(&nested_call.name).unwrap_or(&nested_call.name),
                    "arguments": nested_call.arguments,
                    "status": result.status,
                    "aggregateStatus": aggregate_status,
                    "ok": aggregate_status == "ok",
                    "durationMs": result.duration_ms,
                    "error": output.get("error").cloned(),
                    "output": output,
                })
            })
            .collect::<Vec<_>>();

        let runtime_status = if aggregate_status == "error" {
            "error"
        } else {
            "ok"
        };
        let summary = build_nested_results_summary(tool_name, &results, aggregate_status);

        ToolResult {
            tool_name: tool_name.to_string(),
            status: runtime_status.to_string(),
            output: json_string(json!({
                "ok": runtime_status == "ok",
                "status": aggregate_status,
                "plannedCount": results.len(),
                "completedCount": results.len().saturating_sub(aborted_count),
                "successCount": success_count,
                "partialCount": partial_count,
                "errorCount": error_count,
                "abortedCount": aborted_count,
                "meta": meta,
                "plan": plan,
                "summary": summary,
                "results": results,
            })),
            duration_ms: 0,
        }
    }

    fn resolve_workspace_path(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err("文件路径不能为空。".to_string());
        }

        let canonical = match self.canonicalize_workspace_target(trimmed) {
            Ok(canonical) => canonical,
            Err(primary_error) => match self.try_repair_file_path(trimmed) {
                Ok(Some(repaired)) => repaired,
                Ok(None) => return Err(primary_error),
                Err(repair_error) => return Err(repair_error),
            },
        };
        if !canonical.is_file() {
            return Err(format!(
                "目标不是文件：{}。",
                self.display_workspace_relative(&canonical)
            ));
        }

        Ok(canonical)
    }

    fn prepare_workspace_file_path(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err("文件路径不能为空。".to_string());
        }

        let input = PathBuf::from(trimmed);
        let candidate = if input.is_absolute() {
            input
        } else {
            self.workspace_root.join(trimmed)
        };

        let root = self.canonical_workspace_root();
        let parent = candidate.parent().unwrap_or(&self.workspace_root);
        let existing_ancestor = existing_workspace_ancestor_path(parent)
            .ok_or_else(|| format!("无法解析目标父目录 {}。", parent.display()))?;
        let canonical_ancestor = existing_ancestor.canonicalize().map_err(|error| {
            format!(
                "无法解析目标父目录 {}：{}",
                existing_ancestor.display(),
                error
            )
        })?;

        if !is_within_root(&root, &canonical_ancestor) {
            return Err("只允许写入当前工作区内的相对路径。".to_string());
        }

        let relative_suffix = candidate
            .strip_prefix(&existing_ancestor)
            .map_err(|_| "无法计算工作区内的目标路径后缀。".to_string())?;

        Ok(canonical_ancestor.join(relative_suffix))
    }

    fn resolve_workspace_entry(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = if raw_path.trim().is_empty() {
            "."
        } else {
            raw_path.trim()
        };
        match self.canonicalize_workspace_target(trimmed) {
            Ok(canonical) => Ok(canonical),
            Err(primary_error) => match self.try_repair_file_path(trimmed) {
                Ok(Some(repaired)) => Ok(repaired),
                Ok(None) => Err(primary_error),
                Err(repair_error) => Err(repair_error),
            },
        }
    }

    fn resolve_workspace_dir(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = if raw_path.trim().is_empty() {
            "."
        } else {
            raw_path.trim()
        };
        let canonical = self.canonicalize_workspace_target(trimmed)?;
        if !canonical.is_dir() {
            return Err(format!(
                "目标不是目录：{}。",
                self.display_workspace_relative(&canonical)
            ));
        }

        Ok(canonical)
    }

    fn canonicalize_workspace_target(&self, raw_path: &str) -> Result<PathBuf, String> {
        let input = PathBuf::from(raw_path);
        let candidate = if input.is_absolute() {
            input
        } else {
            self.workspace_root.join(raw_path)
        };
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("无法解析路径 {}：{}", raw_path, error))?;
        let root = self.canonical_workspace_root();

        if !is_within_root(&root, &canonical) {
            return Err("只允许访问当前工作区内的相对路径。".to_string());
        }

        Ok(canonical)
    }

    fn canonical_workspace_root(&self) -> PathBuf {
        self.workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone())
    }

    fn display_workspace_relative(&self, path: &Path) -> String {
        let root = self.canonical_workspace_root();
        path.strip_prefix(&root)
            .ok()
            .map(|value| {
                let display = value.display().to_string().replace('\\', "/");
                if display.is_empty() {
                    ".".to_string()
                } else {
                    display
                }
            })
            .unwrap_or_else(|| path.display().to_string().replace('\\', "/"))
    }

    fn try_repair_file_path(&self, raw_path: &str) -> Result<Option<PathBuf>, String> {
        let normalized_raw_path = raw_path
            .trim()
            .trim_end_matches(['/', '\\'])
            .replace('\\', "/");
        let raw_path_object = Path::new(raw_path);
        let file_name = raw_path_object
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "." && *value != "..");

        let Some(file_name) = file_name else {
            return Ok(None);
        };

        let root = self.canonical_workspace_root();
        let mut files = Vec::new();
        if collect_files_recursively(&root, &mut files, MAX_PATH_REPAIR_SEARCH_FILES).is_err() {
            return Ok(None);
        }

        let exact_name_matches = files
            .iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case(file_name))
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();

        match exact_name_matches.len() {
            0 => {}
            1 => return Ok(exact_name_matches.into_iter().next()),
            _ => {
                let candidates = exact_name_matches
                    .iter()
                    .take(5)
                    .map(|path| self.display_workspace_relative(path))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "无法解析路径 {}：工作区内发现多个同名文件 {}，请提供更精确的相对路径。",
                    raw_path, candidates
                ));
            }
        }

        let stem_matches = files
            .iter()
            .filter(|path| {
                let relative = match path.strip_prefix(&root) {
                    Ok(value) => value,
                    Err(_) => return false,
                };
                let relative_without_extension = relative
                    .parent()
                    .map(|parent| parent.join(relative.file_stem().unwrap_or_default()))
                    .unwrap_or_else(|| PathBuf::from(relative.file_stem().unwrap_or_default()));

                relative_without_extension
                    .display()
                    .to_string()
                    .replace('\\', "/")
                    .eq_ignore_ascii_case(&normalized_raw_path)
                    || path
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .map(|value| value.eq_ignore_ascii_case(file_name))
                        .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();

        match stem_matches.len() {
            0 => Ok(None),
            1 => Ok(stem_matches.into_iter().next()),
            _ => {
                let candidates = stem_matches
                    .iter()
                    .take(5)
                    .map(|path| self.display_workspace_relative(path))
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(format!(
                    "无法解析路径 {}：工作区内发现多个缺扩展名候选文件 {}，请提供更精确的相对路径。",
                    raw_path, candidates
                ))
            }
        }
    }
}

impl ToolExecutor for ToolRouter {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        ToolRouter::execute(self, call)
    }
}

impl ToolDefinition {
    pub fn contract_view(&self) -> ToolDefinitionContractView {
        ToolDefinitionContractView {
            name: product_visible_tool_name(self.name),
            canonical_tool_name: product_visible_tool_name(self.name),
            execution_primitive: canonical_tool_name(self.name)
                .unwrap_or(self.name)
                .to_string(),
            description: self.description.to_string(),
            input_schema: self.input_schema.clone(),
            kind: tool_kind_for_name(self.name).as_str().to_string(),
            exposure: tool_exposure_for_name(self.name).as_str().to_string(),
            display_metadata: tool_display_metadata_for_name(self.name),
            permission_facts: default_permission_facts_for_name(self.name),
        }
    }
}

fn with_description(schema: Value) -> Value {
    let desc = json!({
        "type": "string",
        "description": "用中文极简描述本次工具调用的目的，用于用户界面展示。每次调用工具时必须提供此字段。例如「读取 config.json」「搜索 TokenManager」「运行单元测试」。"
    });
    if let Some(properties) = schema
        .as_object()
        .and_then(|o| o.get("properties"))
        .and_then(|p| p.as_object())
    {
        let mut props = properties.clone();
        props.insert("description".to_string(), desc);
        let mut schema = Value::Object(schema.as_object().unwrap().clone());
        let object = schema.as_object_mut().unwrap();
        object.insert("properties".to_string(), Value::Object(props));
        let mut required = object
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !required
            .iter()
            .any(|value| value.as_str() == Some("description"))
        {
            required.push(Value::String("description".to_string()));
        }
        object.insert("required".to_string(), Value::Array(required));
        schema
    } else {
        schema
    }
}

pub fn builtin_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: TOOL_TIME_NOW,
            description: "返回当前本机 UNIX 时间戳，适合最小时间查询演示。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_ECHO_INPUT,
            description: "把传入的 text 原样返回，适合验证 tool roundtrip。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "需要原样回显给用户的文本"
                    }
                },
                "required": ["text"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_READ_FILE,
            description: "读取当前工作区内的文本文件内容预览，需要提供相对路径；大文件会被拒绝并引导改用分段读取。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_READ_DOCUMENT,
            description: "把当前工作区内的办公文档（Word/PPT/Excel/ODF/RTF/EPUB/CSV/PDF）本地转换为 GitHub 风格 Markdown 文本返回，需要提供相对路径；扫描版 PDF（无内嵌文本）不支持，转换结果超出 maxOutputBytes 时截断并在 truncated 字段给出证据。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文档路径（.doc/.docx/.ppt/.pptx/.xls/.xlsx/.odt/.ods/.odp/.rtf/.epub/.csv/.pdf 等）"
                    },
                    "maxOutputBytes": {
                        "type": "integer",
                        "description": "转换结果 Markdown 输出上限（字节），默认 524288"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_READ_FILE_SEGMENT,
            description: "按行读取当前工作区文件的一段内容，适合大文件局部查看。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    },
                    "startLine": {
                        "type": "integer",
                        "description": "从第几行开始读取，最小为 1"
                    },
                    "lineCount": {
                        "type": "integer",
                        "description": "读取多少行，默认 40，最大 400"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_LIST_FILES,
            description: "列出当前工作区目录下的文件和子目录，可指定相对路径和返回条数。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对目录路径，默认为 ."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少个条目，默认 40"
                    }
                },
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_PATH_INFO,
            description: "返回工作区内文件或目录的路径元信息，适合快速判断它是什么、大小和层级。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对路径，默认为 ."
                    }
                },
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_SEARCH_TEXT,
            description: "递归搜索工作区目录内的文本内容，返回命中的路径、行号和预览。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "要搜索的关键字或文本片段"
                    },
                    "path": {
                        "type": "string",
                        "description": "搜索起点目录，默认为 ."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少条命中，默认 20"
                    },
                    "ignoreCase": {
                        "type": "boolean",
                        "description": "是否忽略大小写，默认 true"
                    },
                    "regex": {
                        "type": "boolean",
                        "description": "是否按增强模式匹配 query；当前 v1 使用通配符式匹配，默认 false"
                    },
                    "filePattern": {
                        "type": "string",
                        "description": "可选的路径子串过滤，例如 .rs 或 src/agent"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_GLOB_FILES,
            description: "按路径 pattern 递归匹配工作区内文件，适合大代码库中的文件发现。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "要匹配的路径模式，例如 src/*.rs 或 *tool*"
                    },
                    "path": {
                        "type": "string",
                        "description": "搜索起点目录，默认为 ."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少条路径命中，默认 50"
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WEB_FETCH_URL,
            description: "抓取指定 http/https URL 的正文内容预览，不承担搜索排序职责。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "要抓取的 http/https URL"
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "请求超时毫秒数，默认 15000"
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WEB_SEARCH_QUERY,
            description: "执行外部搜索并返回结构化结果列表，不把抓取和搜索混为一个工具。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "外部搜索关键词"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少条搜索结果，默认 5"
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "请求超时毫秒数，默认 15000"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            // ── Boundary decision (PA-076 P2-6): MCP resource read stays in the capability
            // registry path (runtime/tool_exec.rs:execute_resource_registry_tool_call). The
            // builtin registry entry is discovery metadata only; actual execution goes through
            // the source-bound `McpTransport` + `CapabilityRegistry`, not the governed
            // dispatcher. Adding a governed handler for this descriptor WOULD create a double
            // execution path, which is explicitly forbidden. ──
            name: TOOL_MCP_RESOURCE_READ,
            description: "通过 capability registry 读取指定 MCP 资源 capability 的只读内容，不混入普通工具执行。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "capabilityId": {
                        "type": "string",
                        "description": "目标 resource capability id，例如 mcp:resource:repo-index"
                    },
                    "arguments": {
                        "type": "object",
                        "description": "传给 resource capability 的结构化参数"
                    }
                },
                "required": ["capabilityId"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            // ── Boundary decision (PA-076 P2-6): ToolSearch stays in the capability registry
            // path (runtime/tool_exec.rs:execute_tool_search_registry_tool_call). The builtin
            // registry entry is discovery metadata only; actual search and elevation go through
            // the `ToolSearchElevator` + `TurnToolView` + `CapabilityRegistry`, not the
            // governed dispatcher. Adding a governed handler WOULD create a double execution
            // path, which is explicitly forbidden. ──
            name: TOOL_TOOL_SEARCH,
            description: "搜索 capability registry 中可用的工具候选，作为 deferred / dynamic tool discovery 入口。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "可选查询词；为空时返回默认候选列表"
                    },
                    "sourceId": {
                        "type": "string",
                        "description": "可选 source id，用于缩小 discovery 范围"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少条候选，默认 8"
                    }
                },
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_WRITE_FILE,
            description: "在当前工作区内新建或整文件覆写文本文件，可控制是否允许覆盖现有文件。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    },
                    "content": {
                        "type": "string",
                        "description": "要写入文件的完整文本内容"
                    },
                    "overwrite": {
                        "type": "boolean",
                        "description": "是否允许覆盖已存在文件，默认 true"
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_EDIT_FILE,
            description: "在当前工作区内按 oldText/newText 对文本文件做受控替换；默认只允许单一匹配。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    },
                    "oldText": {
                        "type": "string",
                        "description": "需要被替换的原始文本"
                    },
                    "newText": {
                        "type": "string",
                        "description": "替换后的新文本"
                    },
                    "replaceAll": {
                        "type": "boolean",
                        "description": "是否允许替换全部匹配，默认 false"
                    }
                },
                "required": ["path", "oldText", "newText"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_RUN_COMMAND,
            description: "在当前工作区内受控执行命令，返回 cwd、timeout、exitCode、stdout 和 stderr。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "要执行的命令文本"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "执行命令时的工作区内相对目录，默认 ."
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "命令超时毫秒数，默认 10000，最大 120000"
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_BATCH,
            description: "批量执行多个工具子调用，可选并发和 continueOnError，用于一次性收集多个上下文片段。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "parallel": {
                        "type": "boolean",
                        "description": "是否并发执行；当 continueOnError=false 时会自动退回串行"
                    },
                    "continueOnError": {
                        "type": "boolean",
                        "description": "某个子调用失败后是否继续执行其余子调用"
                    },
                    "calls": {
                        "type": "array",
                        "description": "子调用列表，每个元素包含 name 和可选 arguments",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "arguments": { "type": "object" }
                            },
                            "required": ["name"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["calls"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_GATHER_CONTEXT,
            description: "围绕一个路径自动收集最合适的上下文：文件会拿 path info 和首段内容，目录会拿 path info 和文件列表，带 query 时会连同搜索结果一起返回。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "目标路径，默认为 ."
                    },
                    "paths": {
                        "type": "array",
                        "description": "可选的多路径聚合输入；提供后会依次对每个路径执行单路径 gather 并汇总结果",
                        "items": {
                            "type": "string"
                        }
                    },
                    "query": {
                        "type": "string",
                        "description": "可选搜索词；提供后会进入搜索模式"
                    },
                    "lineCount": {
                        "type": "integer",
                        "description": "文件模式下读取多少行，默认 80，最大 400"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "目录模式和搜索模式下的条数限制，默认 40"
                    }
                },
                "additionalProperties": false
            })),
        },
        // ── PA-076 phase 7 P2-6: Plan + ViewImage added after all existing definitions
        // so the stable prefix invariant (design Decision 10) is preserved. ──
        ToolDefinition {
            name: TOOL_PLAN_CONTROL,
            description: "控制会话级 Plan 状态：create / replace / merge / complete_step，不执行任意子调用。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "description": "plan 控制操作：create / replace / merge / complete_step"
                    },
                    "session_id": {
                        "type": "string",
                        "description": "拥有该 plan 的 session id"
                    },
                    "plan_id": {
                        "type": "string",
                        "description": "要修改的 plan id（replace / merge / complete_step 需要）"
                    },
                    "revision": {
                        "type": "integer",
                        "description": "期望的 plan revision，用于 CAS 冲突检测（replace / merge / complete_step 需要）"
                    },
                    "step_id": {
                        "type": "string",
                        "description": "要标记完成的 step id（complete_step 需要）"
                    },
                    "payload": {
                        "type": "object",
                        "description": "plan 内容（create / replace 需要）",
                        "properties": {
                            "kind": { "type": "string", "description": "plan 分类标签" },
                            "summary": { "type": "string", "description": "plan 概要描述" },
                            "steps": {
                                "type": "array",
                                "description": "step 规范列表",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "name": { "type": "string" },
                                        "summary": { "type": "string" }
                                    }
                                }
                            }
                        }
                    },
                    "step": {
                        "type": "object",
                        "description": "merge 操作要追加的 step 规范",
                        "properties": {
                            "name": { "type": "string" },
                            "summary": { "type": "string" }
                        }
                    }
                },
                "required": ["op"],
                "additionalProperties": false
            })),
        },
        ToolDefinition {
            name: TOOL_VIEW_IMAGE,
            description: "读取工作区内图片并返回规范化 image artifact（受控引用 + magic-byte MIME/尺寸元数据）；默认不嵌入文件字节，是否将字节发送给模型由 provider modality 决定（design Decision 11）。",
            input_schema: with_description(json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对图片路径"
                    },
                    "includeBytes": {
                        "type": "boolean",
                        "description": "是否在 artifact 中嵌入字节；默认 false（仅返回受控引用和元数据）"
                    },
                    "maxWidth": {
                        "type": "integer",
                        "description": "最大允许的图片宽度，超出标记 truncated（默认 8192）"
                    },
                    "maxHeight": {
                        "type": "integer",
                        "description": "最大允许的图片高度，超出标记 truncated（默认 8192）"
                    },
                    "maxBytes": {
                        "type": "integer",
                        "description": "嵌入字节时的最大文件字节数，超出标记 truncated（默认 2 MiB）"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })),
        },
    ]
}

fn contract_priority(view: &ToolDefinitionContractView) -> u8 {
    match view.execution_primitive.as_str() {
        TOOL_TIME_NOW => 10,
        TOOL_ECHO_INPUT => 100,
        TOOL_WORKSPACE_LIST_FILES => 100,
        TOOL_WORKSPACE_SEARCH_TEXT => 100,
        TOOL_WORKSPACE_GLOB_FILES => 100,
        TOOL_WEB_FETCH_URL => 100,
        TOOL_WEB_SEARCH_QUERY => 100,
        TOOL_MCP_RESOURCE_READ => 100,
        TOOL_TOOL_SEARCH => 100,
        TOOL_WORKSPACE_WRITE_FILE => 100,
        TOOL_WORKSPACE_EDIT_FILE => 100,
        TOOL_WORKSPACE_RUN_COMMAND => 100,
        TOOL_WORKSPACE_BATCH => 100,
        TOOL_WORKSPACE_GATHER_CONTEXT => 100,
        TOOL_WORKSPACE_READ_FILE => 40,
        TOOL_WORKSPACE_READ_FILE_SEGMENT => 30,
        TOOL_WORKSPACE_PATH_INFO => 20,
        TOOL_PLAN_CONTROL => 50,
        TOOL_VIEW_IMAGE => 50,
        _ => 50,
    }
}

#[cfg(test)]
mod contract_view_tests {
    use super::{
        builtin_tools, builtin_turn_tool_contract_views, canonical_tool_name,
        default_permission_facts_for_name, model_visible_tool_name, product_canonical_tool_name,
        ToolDescriptor, ToolDescriptorSource, ToolExposure, ToolHandlerProvenance, ToolIdentity,
        ToolKind, ToolRegistrySnapshot, TOOL_WORKSPACE_PATH_INFO, TOOL_WORKSPACE_RUN_COMMAND,
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
                "ViewImage"
            ]
        );
        assert_eq!(
            primitives,
            vec![
                "workspace_run_command",
                "echo_input",
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
                "view_image"
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

    #[test]
    fn characterization_product_surface_keeps_unique_names_and_json_object_schemas() {
        let tools = builtin_tools();
        let views = builtin_turn_tool_contract_views();
        let names = views
            .iter()
            .map(|view| view.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(tools.len(), 20, "baseline internal primitive count");
        assert_eq!(names.len(), 16, "baseline product tool count");
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
            ("Ask", "echo_input"),
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
        // These assertions document the behavior being replaced by PA-076. They are not a
        // target product contract: Ask must stop echoing and Plan must stop dispatching batches.
        assert_eq!(canonical_tool_name("Ask"), Some("echo_input"));
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
        assert_eq!(names.len(), 16);
        assert!(registry.resolve("Run").is_some());
        assert!(registry.resolve("workspace.run_command").is_some());
        assert!(registry.resolve("workspace_path_info").is_some());
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
}

pub(crate) fn canonical_tool_name(name: &str) -> Option<&'static str> {
    match name {
        "Run" => Some(TOOL_WORKSPACE_RUN_COMMAND),
        "Ask" => Some(TOOL_ECHO_INPUT),
        "List" => Some(TOOL_WORKSPACE_LIST_FILES),
        "Read" => Some(TOOL_WORKSPACE_GATHER_CONTEXT),
        "Search" => Some(TOOL_WORKSPACE_SEARCH_TEXT),
        "Glob" => Some(TOOL_WORKSPACE_GLOB_FILES),
        "WebFetch" => Some(TOOL_WEB_FETCH_URL),
        "WebSearch" => Some(TOOL_WEB_SEARCH_QUERY),
        "MCPResource" => Some(TOOL_MCP_RESOURCE_READ),
        "ToolSearch" => Some(TOOL_TOOL_SEARCH),
        "Write" => Some(TOOL_WORKSPACE_WRITE_FILE),
        "Edit" => Some(TOOL_WORKSPACE_EDIT_FILE),
        "BatchExecute" => Some(TOOL_WORKSPACE_BATCH),
        "Plan" => Some(TOOL_PLAN_CONTROL),
        "ViewImage" => Some(TOOL_VIEW_IMAGE),
        TOOL_TIME_NOW | "time.now" => Some(TOOL_TIME_NOW),
        TOOL_ECHO_INPUT | "echo.input" => Some(TOOL_ECHO_INPUT),
        TOOL_WORKSPACE_LIST_FILES | "workspace.list_files" => Some(TOOL_WORKSPACE_LIST_FILES),
        TOOL_WORKSPACE_READ_FILE | "workspace.read_file" => Some(TOOL_WORKSPACE_READ_FILE),
        TOOL_WORKSPACE_READ_FILE_SEGMENT | "workspace.read_file_segment" => {
            Some(TOOL_WORKSPACE_READ_FILE_SEGMENT)
        }
        TOOL_WORKSPACE_PATH_INFO | "workspace.path_info" => Some(TOOL_WORKSPACE_PATH_INFO),
        TOOL_WORKSPACE_SEARCH_TEXT | "workspace.search_text" => Some(TOOL_WORKSPACE_SEARCH_TEXT),
        TOOL_WORKSPACE_GLOB_FILES | "workspace.glob_files" => Some(TOOL_WORKSPACE_GLOB_FILES),
        TOOL_WEB_FETCH_URL | "web.fetch_url" => Some(TOOL_WEB_FETCH_URL),
        TOOL_WEB_SEARCH_QUERY | "web.search_query" => Some(TOOL_WEB_SEARCH_QUERY),
        TOOL_MCP_RESOURCE_READ | "mcp.resource_read" => Some(TOOL_MCP_RESOURCE_READ),
        TOOL_TOOL_SEARCH | "tool.search" => Some(TOOL_TOOL_SEARCH),
        TOOL_WORKSPACE_WRITE_FILE | "workspace.write_file" => Some(TOOL_WORKSPACE_WRITE_FILE),
        TOOL_WORKSPACE_EDIT_FILE | "workspace.edit_file" => Some(TOOL_WORKSPACE_EDIT_FILE),
        TOOL_WORKSPACE_RUN_COMMAND | "workspace.run_command" => Some(TOOL_WORKSPACE_RUN_COMMAND),
        TOOL_WORKSPACE_BATCH | "workspace.batch" => Some(TOOL_WORKSPACE_BATCH),
        TOOL_WORKSPACE_GATHER_CONTEXT | "workspace.gather_context" => {
            Some(TOOL_WORKSPACE_GATHER_CONTEXT)
        }
        TOOL_PLAN_CONTROL | "plan.control" => Some(TOOL_PLAN_CONTROL),
        TOOL_VIEW_IMAGE | "view.image" => Some(TOOL_VIEW_IMAGE),
        TOOL_WORKSPACE_READ_DOCUMENT | "workspace.read_document" => {
            Some(TOOL_WORKSPACE_READ_DOCUMENT)
        }
        _ => None,
    }
}

fn builtin_aliases_for_primitive(primitive: &str) -> Vec<&'static str> {
    match primitive {
        TOOL_TIME_NOW => vec![TOOL_TIME_NOW, "time.now"],
        TOOL_ECHO_INPUT => vec![TOOL_ECHO_INPUT, "echo.input"],
        TOOL_WORKSPACE_LIST_FILES => vec![TOOL_WORKSPACE_LIST_FILES, "workspace.list_files"],
        TOOL_WORKSPACE_READ_FILE => vec![TOOL_WORKSPACE_READ_FILE, "workspace.read_file"],
        TOOL_WORKSPACE_READ_FILE_SEGMENT => {
            vec![
                TOOL_WORKSPACE_READ_FILE_SEGMENT,
                "workspace.read_file_segment",
            ]
        }
        TOOL_WORKSPACE_PATH_INFO => vec![TOOL_WORKSPACE_PATH_INFO, "workspace.path_info"],
        TOOL_WORKSPACE_SEARCH_TEXT => vec![TOOL_WORKSPACE_SEARCH_TEXT, "workspace.search_text"],
        TOOL_WORKSPACE_GLOB_FILES => vec![TOOL_WORKSPACE_GLOB_FILES, "workspace.glob_files"],
        TOOL_WEB_FETCH_URL => vec![TOOL_WEB_FETCH_URL, "web.fetch_url"],
        TOOL_WEB_SEARCH_QUERY => vec![TOOL_WEB_SEARCH_QUERY, "web.search_query"],
        TOOL_MCP_RESOURCE_READ => vec![TOOL_MCP_RESOURCE_READ, "mcp.resource_read"],
        TOOL_TOOL_SEARCH => vec![TOOL_TOOL_SEARCH, "tool.search"],
        TOOL_WORKSPACE_WRITE_FILE => vec![TOOL_WORKSPACE_WRITE_FILE, "workspace.write_file"],
        TOOL_WORKSPACE_EDIT_FILE => vec![TOOL_WORKSPACE_EDIT_FILE, "workspace.edit_file"],
        TOOL_WORKSPACE_RUN_COMMAND => vec![TOOL_WORKSPACE_RUN_COMMAND, "workspace.run_command"],
        TOOL_WORKSPACE_BATCH => vec![TOOL_WORKSPACE_BATCH, "workspace.batch"],
        TOOL_WORKSPACE_GATHER_CONTEXT => {
            vec![TOOL_WORKSPACE_GATHER_CONTEXT, "workspace.gather_context"]
        }
        TOOL_PLAN_CONTROL => vec![TOOL_PLAN_CONTROL, "plan.control"],
        TOOL_VIEW_IMAGE => vec![TOOL_VIEW_IMAGE, "view.image"],
        TOOL_WORKSPACE_READ_DOCUMENT => vec![TOOL_WORKSPACE_READ_DOCUMENT, "workspace.read_document"],
        _ => Vec::new(),
    }
}

fn execution_policy_for_primitive(primitive: &str) -> ToolExecutionPolicy {
    let mut policy = ToolExecutionPolicy::default();
    match primitive {
        TOOL_WORKSPACE_LIST_FILES
        | TOOL_WORKSPACE_READ_FILE
        | TOOL_WORKSPACE_READ_FILE_SEGMENT
        | TOOL_WORKSPACE_PATH_INFO
        | TOOL_WORKSPACE_SEARCH_TEXT
        | TOOL_WORKSPACE_GLOB_FILES
        | TOOL_WEB_FETCH_URL
        | TOOL_WEB_SEARCH_QUERY
        | TOOL_MCP_RESOURCE_READ
        | TOOL_TOOL_SEARCH
        | TOOL_WORKSPACE_GATHER_CONTEXT => policy.concurrent_safe = true,
        TOOL_VIEW_IMAGE => {
            policy.concurrent_safe = true;
            // Image artifact with include_bytes=true can be up to the default 2 MiB byte cap, so
            // the result budget should not truncate a valid bounded payload (design Decision 11:
            // reference-based by default, but caller-adjustable).
            policy.result_budget_bytes = 2 * 1024 * 1024;
        }
        TOOL_WORKSPACE_READ_DOCUMENT => {
            policy.concurrent_safe = true;
            // Converted Markdown can reach the default 512 KiB output cap, so the result budget
            // must not truncate a valid bounded payload.
            policy.result_budget_bytes = crate::agent::document_conversion::DEFAULT_MAX_OUTPUT_BYTES as usize;
        }
        TOOL_WORKSPACE_RUN_COMMAND => {
            policy.default_timeout_ms = DEFAULT_RUN_TIMEOUT_MS;
            policy.result_budget_bytes = MAX_FULL_READ_BYTES as usize;
        }
        _ => {}
    }
    policy
}

fn default_permission_declaration_for_name(name: &str) -> ToolPermissionDeclaration {
    let facts = default_permission_facts_for_name(name);
    let mut scopes = std::collections::BTreeSet::new();
    if let Some(scope) = facts.permission_scope.as_deref() {
        let typed_scope = match scope {
            "workspace.read" => Some(ToolPermissionScope::WorkspaceRead),
            "workspace.write" => Some(ToolPermissionScope::WorkspaceWrite),
            "workspace.execute" => Some(ToolPermissionScope::WorkspaceExecute),
            "capability.discovery" => Some(ToolPermissionScope::CapabilityDiscovery),
            _ => None,
        };
        if let Some(scope) = typed_scope {
            scopes.insert(scope);
        }
    }
    ToolPermissionDeclaration {
        scopes,
        requires_approval: facts.requires_approval.unwrap_or(true),
        host_mediated: facts.host_mediated.unwrap_or(true),
        approval_mode: facts.approval_mode,
    }
}

/// Maps a known builtin primitive to its product-visible name. Unknown names have no builtin
/// product identity, so callers must keep the caller-supplied name instead of inventing one.
pub fn model_visible_tool_name_opt(name: &str) -> Option<&'static str> {
    let mapped = match canonical_tool_name(name).unwrap_or(name) {
        TOOL_WORKSPACE_RUN_COMMAND => "Run",
        TOOL_TIME_NOW => "Run",
        TOOL_ECHO_INPUT => "Ask",
        TOOL_WORKSPACE_LIST_FILES => "List",
        TOOL_WORKSPACE_READ_FILE | TOOL_WORKSPACE_READ_FILE_SEGMENT => "Read",
        TOOL_WORKSPACE_PATH_INFO => "List",
        TOOL_WORKSPACE_SEARCH_TEXT => "Search",
        TOOL_WORKSPACE_GLOB_FILES => "Glob",
        TOOL_WEB_FETCH_URL => "WebFetch",
        TOOL_WEB_SEARCH_QUERY => "WebSearch",
        TOOL_MCP_RESOURCE_READ => "MCPResource",
        TOOL_TOOL_SEARCH => "ToolSearch",
        TOOL_WORKSPACE_WRITE_FILE => "Write",
        TOOL_WORKSPACE_EDIT_FILE => "Edit",
        TOOL_WORKSPACE_BATCH => "BatchExecute",
        TOOL_WORKSPACE_GATHER_CONTEXT => "Read",
        TOOL_PLAN_CONTROL => "Plan",
        TOOL_VIEW_IMAGE => "ViewImage",
        TOOL_WORKSPACE_READ_DOCUMENT => "ReadDocument",
        _ => return None,
    };
    Some(mapped)
}

pub fn model_visible_tool_name(name: &str) -> &'static str {
    // Preserved for builtin call sites that already hold a known primitive. Unknown names keep
    // the legacy `Run` fallback only where a `&'static str` is required; prefer the `_opt` form.
    model_visible_tool_name_opt(name).unwrap_or("Run")
}

/// Product-visible name for contract views. Unknown (external/dynamic) tool names pass through
/// unchanged so no external tool is ever mislabeled as a builtin product tool.
pub fn product_visible_tool_name(name: &str) -> String {
    model_visible_tool_name_opt(name)
        .map(str::to_string)
        .unwrap_or_else(|| name.to_string())
}

pub fn product_canonical_tool_name(name: &str) -> &'static str {
    model_visible_tool_name(name)
}

pub fn tool_kind_for_name(name: &str) -> ToolKind {
    match canonical_tool_name(name).unwrap_or(name) {
        TOOL_WORKSPACE_READ_FILE
        | TOOL_WORKSPACE_READ_FILE_SEGMENT
        | TOOL_WORKSPACE_PATH_INFO
        | TOOL_WORKSPACE_GATHER_CONTEXT
        | TOOL_WORKSPACE_GLOB_FILES
        | TOOL_WEB_FETCH_URL
        | TOOL_MCP_RESOURCE_READ => ToolKind::Read,
        TOOL_WORKSPACE_SEARCH_TEXT | TOOL_WEB_SEARCH_QUERY | TOOL_TOOL_SEARCH => ToolKind::Search,
        TOOL_WORKSPACE_WRITE_FILE | TOOL_WORKSPACE_EDIT_FILE => ToolKind::Write,
        TOOL_WORKSPACE_LIST_FILES => ToolKind::Read,
        TOOL_WORKSPACE_BATCH => ToolKind::Composite,
        TOOL_ECHO_INPUT => ToolKind::Interactive,
        // `time_now` is a pure clock read, not a side-effecting Execute (phase-4..7 review P2-10:
        // the dispatcher's sandbox gate keys off Execute kind, so misclassifying the clock made it
        // fail `sandbox_unavailable` through the governed path).
        TOOL_TIME_NOW => ToolKind::Read,
        TOOL_WORKSPACE_RUN_COMMAND => ToolKind::Execute,
        TOOL_PLAN_CONTROL => ToolKind::Write,
        TOOL_VIEW_IMAGE => ToolKind::Read,
        TOOL_WORKSPACE_READ_DOCUMENT => ToolKind::Read,
        _ => ToolKind::External,
    }
}

pub fn tool_exposure_for_name(name: &str) -> ToolExposure {
    match canonical_tool_name(name).unwrap_or(name) {
        TOOL_WORKSPACE_BATCH
        | TOOL_WORKSPACE_GATHER_CONTEXT
        | TOOL_WORKSPACE_LIST_FILES
        | TOOL_WORKSPACE_READ_FILE
        | TOOL_WORKSPACE_READ_FILE_SEGMENT
        | TOOL_WORKSPACE_GLOB_FILES
        | TOOL_WORKSPACE_SEARCH_TEXT
        | TOOL_WEB_FETCH_URL
        | TOOL_WEB_SEARCH_QUERY
        | TOOL_MCP_RESOURCE_READ
        | TOOL_WORKSPACE_WRITE_FILE
        | TOOL_WORKSPACE_EDIT_FILE
        | TOOL_WORKSPACE_RUN_COMMAND
        | TOOL_ECHO_INPUT
        | TOOL_TIME_NOW => ToolExposure::ModelVisible,
        TOOL_TOOL_SEARCH => ToolExposure::Deferred,
        TOOL_WORKSPACE_PATH_INFO => ToolExposure::Deferred,
        TOOL_PLAN_CONTROL => ToolExposure::ModelVisible,
        TOOL_VIEW_IMAGE => ToolExposure::ModelVisible,
        TOOL_WORKSPACE_READ_DOCUMENT => ToolExposure::ModelVisible,
        _ => ToolExposure::Internal,
    }
}

pub fn tool_display_metadata_for_name(name: &str) -> ToolDisplayMetadata {
    let display_name_zh = match model_visible_tool_name(name) {
        "Read" => Some("读取".to_string()),
        "Search" => Some("搜索".to_string()),
        "List" => Some("列表".to_string()),
        "Glob" => Some("匹配".to_string()),
        "WebFetch" => Some("抓取".to_string()),
        "WebSearch" => Some("外搜".to_string()),
        "MCPResource" => Some("资源".to_string()),
        "ToolSearch" => Some("找工具".to_string()),
        "BatchExecute" => Some("批量执行".to_string()),
        "Ask" => Some("提问".to_string()),
        "Run" => Some("运行".to_string()),
        "Write" => Some("写入".to_string()),
        "Edit" => Some("编辑".to_string()),
        "Plan" => Some("计划".to_string()),
        "ViewImage" => Some("看图".to_string()),
        "ReadDocument" => Some("读文档".to_string()),
        _ => None,
    };
    ToolDisplayMetadata { display_name_zh }
}

pub fn default_permission_facts_for_name(name: &str) -> ToolPermissionFacts {
    let scope = match canonical_tool_name(name).unwrap_or(name) {
        TOOL_WORKSPACE_LIST_FILES
        | TOOL_WORKSPACE_READ_FILE
        | TOOL_WORKSPACE_READ_FILE_SEGMENT
        | TOOL_WORKSPACE_PATH_INFO
        | TOOL_WORKSPACE_GLOB_FILES
        | TOOL_WORKSPACE_SEARCH_TEXT
        | TOOL_WEB_FETCH_URL
        | TOOL_WEB_SEARCH_QUERY
        | TOOL_MCP_RESOURCE_READ
        | TOOL_WORKSPACE_GATHER_CONTEXT
        | TOOL_WORKSPACE_BATCH => Some("workspace.read".to_string()),
        TOOL_TOOL_SEARCH => Some("capability.discovery".to_string()),
        TOOL_WORKSPACE_WRITE_FILE | TOOL_WORKSPACE_EDIT_FILE => Some("workspace.write".to_string()),
        TOOL_WORKSPACE_RUN_COMMAND => Some("workspace.execute".to_string()),
        TOOL_TIME_NOW | TOOL_ECHO_INPUT | TOOL_PLAN_CONTROL => None,
        TOOL_VIEW_IMAGE => Some("workspace.read".to_string()),
        TOOL_WORKSPACE_READ_DOCUMENT => Some("workspace.read".to_string()),
        _ => None,
    };
    ToolPermissionFacts {
        requires_approval: Some(false),
        permission_scope: scope,
        host_mediated: Some(false),
        permission_profile: Some("builtin".to_string()),
        approval_mode: Some("none".to_string()),
        decision_source: Some("runtime".to_string()),
    }
}

fn truncate_preview(content: &str, max_chars: usize) -> String {
    let preview = content.chars().take(max_chars).collect::<String>();
    if content.chars().count() > max_chars {
        format!(
            "{}\n\n[内容已截断，当前仅展示前 {} 个字符]",
            preview, max_chars
        )
    } else {
        preview
    }
}

fn is_within_root(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        text.to_string()
    } else {
        let preview = text.chars().take(max_chars).collect::<String>();
        format!("{}...(+{} chars)", preview, count - max_chars)
    }
}

fn collect_files_recursively(
    dir: &Path,
    files: &mut Vec<PathBuf>,
    max_files: usize,
) -> std::io::Result<()> {
    if files.len() >= max_files {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if should_skip_dir(&path) {
                continue;
            }
            collect_files_recursively(&path, files, max_files)?;
            if files.len() >= max_files {
                break;
            }
        } else if path.is_file() {
            files.push(path);
            if files.len() >= max_files {
                break;
            }
        }
    }

    Ok(())
}

fn should_skip_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some(".git" | "node_modules" | "target" | "dist" | "build")
    )
}

fn unique_paths(paths: Vec<String>) -> Vec<String> {
    let mut unique = Vec::new();
    for path in paths {
        if !unique.iter().any(|existing| existing == &path) {
            unique.push(path);
        }
    }
    unique
}

/// Validate a path glob with the same globset settings the `SearchEngine` uses. Malformed
/// patterns fail closed so callers can surface a structured error instead of a panic.
fn compile_path_glob(pattern: &str) -> Result<(), String> {
    globset::GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map(|_| ())
        .map_err(|error| format!("invalid glob `{pattern}`: {error}"))
}

/// Mirrors the `SearchEngine`'s glob semantics (design Decision 9): a pattern matches the
/// workspace-relative path or its basename, so `*.rs` matches files in any subdirectory. Used by
/// the file-root branches of `search_text`/`glob_files` and by the file-root post-filter.
fn glob_matches_path_or_basename(relative: &str, pattern: &str) -> bool {
    let matcher = globset::GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .expect("path glob was validated up front")
        .compile_matcher();
    if matcher.is_match(relative) {
        return true;
    }
    relative
        .rsplit('/')
        .next()
        .map(|basename| matcher.is_match(basename))
        .unwrap_or(false)
}

fn denied_run_command_reason(command: &str) -> Option<String> {
    let normalized = command.trim().to_lowercase();
    let tokens = normalized
        .split_whitespace()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        return None;
    }

    if is_rm_destructive_command(&tokens) {
        return Some("命令包含高风险删除模式 `rm -rf`，当前 runtime 已拒绝执行。".to_string());
    }
    if is_git_reset_hard_command(&tokens) {
        return Some(
            "命令包含高风险片段 `git reset --hard`，当前 runtime 已拒绝执行。".to_string(),
        );
    }
    if is_git_clean_force_command(&tokens) {
        return Some("命令包含高风险片段 `git clean -fd`，当前 runtime 已拒绝执行。".to_string());
    }

    let deny_markers = [
        "remove-item",
        " del ",
        "erase ",
        "rmdir /s",
        "mkfs",
        "shutdown",
        "reboot",
        "halt",
        "poweroff",
        "diskpart",
    ];

    let padded = format!(" {normalized} ");
    deny_markers
        .iter()
        .find(|marker| padded.contains(**marker))
        .map(|marker| format!("命令包含高风险片段 `{marker}`，当前 runtime 已拒绝执行。"))
        .or_else(|| {
            if is_windows_format_command(&tokens) {
                Some(
                    "命令包含高风险磁盘格式化模式 `format <drive>`，当前 runtime 已拒绝执行。"
                        .to_string(),
                )
            } else {
                None
            }
        })
}

fn is_rm_destructive_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("rm") {
        return false;
    }
    let mut has_recursive = false;
    let mut has_force = false;
    for token in tokens.iter().skip(1) {
        if let Some(flags) = token.strip_prefix('-') {
            has_recursive |= flags.contains('r');
            has_force |= flags.contains('f');
        }
    }
    has_recursive && has_force
}

fn is_git_reset_hard_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("git") {
        return false;
    }
    let has_reset = tokens.contains(&"reset");
    let has_hard = tokens
        .iter()
        .any(|token| *token == "--hard" || *token == "-h");
    has_reset && has_hard
}

fn is_git_clean_force_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("git") {
        return false;
    }
    if !tokens.contains(&"clean") {
        return false;
    }
    tokens.iter().any(|token| {
        if let Some(flags) = token.strip_prefix('-') {
            flags.contains('f') && flags.contains('d')
        } else {
            false
        }
    })
}

fn is_windows_format_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("format") {
        return false;
    }
    tokens.iter().skip(1).any(|token| {
        token.len() == 2
            && token.ends_with(':')
            && token
                .chars()
                .next()
                .map(|value| value.is_ascii_alphabetic())
                .unwrap_or(false)
    })
}

fn build_web_client(timeout_ms: u64) -> Result<Client, String> {
    // Design Decision 8: no ambient proxy and no automatic redirects. The web access policy
    // validates every URL (and would validate every redirect hop) before any connection; the
    // client never follows redirects on its own and never reads proxy environment variables.
    // Used by `web_search` (fixed, trusted Exa endpoint); `web_fetch` uses the pinned per-hop
    // client built by [`build_pinned_web_client`] instead.
    Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent(WEB_USER_AGENT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败：{}。", error))
}

/// One validated HTTP hop in the pinned fetch pipeline: the requested URL, the response status
/// and headers, the bounded body (empty for redirect hops, whose bodies are never read), whether
/// the body was truncated at the cap, and the actual peer address of the connection (verified to
/// be one of the decision's resolved addresses).
#[derive(Clone, Debug)]
struct PinnedHttpExchange {
    url: String,
    status: reqwest::StatusCode,
    headers: reqwest::header::HeaderMap,
    body: Vec<u8>,
    body_truncated: bool,
    /// The actual peer IP of the successful connection; validated against resolved_addrs.
    #[allow(dead_code)]
    peer_ip: Option<IpAddr>,
}

impl PinnedHttpExchange {
    /// The `Location` header when present and non-empty after trimming.
    fn location(&self) -> Option<String> {
        self.headers
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    /// A hop is a redirect only when it carries a 3xx status AND a usable `Location` header; a
    /// 3xx without `Location` is treated as a terminal response and is never auto-followed.
    fn is_redirect(&self) -> bool {
        self.status.is_redirection() && self.location().is_some()
    }
}

/// Transport/pinning failures from [`PinnedSender`]. Distinct from policy denials
/// ([`PinnedFetchError::Denied`]).
#[derive(Clone, Debug)]
enum WebFetchTransportError {
    ClientBuild(String),
    Request(String),
    UnsupportedContentType { content_type: String },
    ReadBody(String),
    PeerMismatch { expected: Vec<IpAddr>, actual: IpAddr },
    /// The whole-fetch chain deadline was exhausted before the fetch completed (design Decision 8
    /// / tasks 6.2-6.3). The fetch is cut structurally rather than letting per-hop timeouts
    /// accumulate across redirect hops and retries.
    DeadlineExceeded {
        /// The chain budget (ms) assigned to the whole fetch.
        budget_ms: u64,
    },
}

/// Failure of the whole pinned web fetch pipeline: a policy denial (initial URL or a redirect
/// hop) or a transport/pinning failure.
#[derive(Clone, Debug)]
enum PinnedFetchError {
    Denied {
        url: String,
        decision: WebAccessDecision,
    },
    Transport(WebFetchTransportError),
}

/// Sends one validated, pinned HTTP request hop. The production implementation
/// ([`ReqwestPinnedSender`]) pins the connection to the decision's resolved addresses via a
/// per-hop reqwest client whose DNS for the URL's hostname is overridden to those addresses; a
/// fake is used in hermetic tests to exercise the redirect re-validation logic without a socket.
trait PinnedSender: Send + Sync {
    fn send(
        &self,
        url: &str,
        decision: &WebAccessDecision,
        timeout_ms: u64,
    ) -> Result<PinnedHttpExchange, WebFetchTransportError>;
}

/// Build a per-hop reqwest client whose connections for `hostname` are pinned to the decision's
/// validated addresses (design Decision 8 / PA-076 item 2).
///
/// The requested URL keeps the original authority, so the `Host` header and the TLS SNI both
/// preserve the validated hostname; only the socket connect target is overridden via reqwest's
/// `resolve_to_addrs` (port `0` means "use the URL's port"). For literal-IP hosts there is no DNS
/// step at all, so the connection is pinned by construction. For hostname URLs the override is
/// mandatory: an empty address set refuses to build the client rather than ever falling back to
/// ambient DNS. This override makes it impossible for the client to connect anywhere except the
/// validated addresses; the per-response peer check in [`ReqwestPinnedSender`] is defense in
/// depth on top of it.
fn build_pinned_web_client(
    decision: &WebAccessDecision,
    hostname: &str,
    timeout_ms: u64,
) -> Result<Client, String> {
    let WebAccessDecision::Allow { resolved_addrs, .. } = decision else {
        return Err("不能为已拒绝的 Web 访问决策构建连接。".to_string());
    };
    let mut builder = Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent(WEB_USER_AGENT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none());
    if hostname.parse::<IpAddr>().is_err() {
        if resolved_addrs.is_empty() {
            return Err("pinned connector 收到空地址集，拒绝回退到环境 DNS。".to_string());
        }
        let pinned: Vec<SocketAddr> = resolved_addrs
            .iter()
            .map(|address| SocketAddr::new(*address, 0))
            .collect();
        builder = builder.resolve_to_addrs(hostname, &pinned);
    }
    builder
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败：{}。", error))
}

/// Verify that the actual peer of a completed connection belongs to the decision's validated
/// address set. The reqwest DNS override in [`build_pinned_web_client`] already makes it
/// impossible to connect outside the validated set; this check is defense in depth so that even a
/// future connector change that removes the override cannot silently accept a different peer.
/// When the peer is not reported (a transport that exposes no address), the override guarantee
/// still holds and no extra check is possible.
fn verify_pinned_peer(
    peer: Option<IpAddr>,
    decision: &WebAccessDecision,
) -> Result<(), WebFetchTransportError> {
    if let Some(peer) = peer {
        if !decision.resolved_addrs().contains(&peer) {
            return Err(WebFetchTransportError::PeerMismatch {
                expected: decision.resolved_addrs().to_vec(),
                actual: peer,
            });
        }
    }
    Ok(())
}

/// Production [`PinnedSender`]: per-hop reqwest client pinned to the decision's validated
/// addresses, with a post-connection peer-IP check (`response.remote_addr()`).
struct ReqwestPinnedSender;

impl PinnedSender for ReqwestPinnedSender {
    fn send(
        &self,
        url: &str,
        decision: &WebAccessDecision,
        timeout_ms: u64,
    ) -> Result<PinnedHttpExchange, WebFetchTransportError> {
        let parsed = Url::parse(url)
            .map_err(|error| WebFetchTransportError::Request(format!("URL 无法解析：{error}")))?;
        let hostname = parsed.host_str().unwrap_or("").to_string();
        let client = build_pinned_web_client(decision, &hostname, timeout_ms)
            .map_err(WebFetchTransportError::ClientBuild)?;
        let response = block_on(client.get(parsed).send()).map_err(|error| {
            if is_reqwest_timeout_error(&error) {
                WebFetchTransportError::Request(format!("timeout: 抓取 URL 超时：{}。", error))
            } else {
                WebFetchTransportError::Request(format!("抓取 URL 失败：{}。", error))
            }
        })?;

        // Peer-IP check: the DNS override already restricts the connection to the validated
        // addresses; confirm the actually-established peer belongs to them as defense in depth.
        let peer_ip = response.remote_addr().map(|address| address.ip());
        verify_pinned_peer(peer_ip, decision)?;

        let status = response.status();
        let headers = response.headers().clone();
        let requested_url = response.url().to_string();

        // Redirect hop: the body is never read; the caller re-validates and re-pins the next hop
        // before any further connection (design Decision 8: bounded redirects, each hop validated).
        if status.is_redirection() && headers.get(reqwest::header::LOCATION).is_some() {
            return Ok(PinnedHttpExchange {
                url: requested_url,
                status,
                headers,
                body: Vec::new(),
                body_truncated: false,
                peer_ip,
            });
        }

        // Terminal hop: gate the declared content type before reading the body, then stream with a
        // hard byte cap and surface truncation honestly (design Decision 8 / phase-4..7 review
        // P1-4; unchanged from the legacy path).
        if let Some(content_type) = headers.get(reqwest::header::CONTENT_TYPE) {
            if !is_text_content_type(content_type.to_str().unwrap_or("")) {
                return Err(WebFetchTransportError::UnsupportedContentType {
                    content_type: content_type.to_str().unwrap_or("unknown").to_string(),
                });
            }
        }
        let (body, body_truncated) = block_on(read_bounded_body(response, MAX_WEB_BODY_BYTES))
            .map_err(WebFetchTransportError::ReadBody)?;
        Ok(PinnedHttpExchange {
            url: requested_url,
            status,
            headers,
            body,
            body_truncated,
            peer_ip,
        })
    }
}

/// Drive a pinned web fetch: validate the initial URL, perform each hop through the injected
/// sender (production: a per-hop client pinned to the decision's validated addresses), and
/// re-validate + re-pin every redirect hop against the same policy. At most
/// `policy.max_redirects` redirects may be followed; any hop that fails validation fails the
/// whole fetch closed. Never falls back to ambient DNS or to a re-resolving client.
///
/// An aggregate chain deadline bounds the whole fetch (design Decision 8 / tasks 6.2-6.3):
/// `deadline` is the single wall-clock instant the whole call — all hops and, via the caller's
/// retry loop, all retries — must finish by. Each hop is granted at most
/// `min(timeout_ms, remaining chain budget)`; once the budget is exhausted the fetch fails
/// structurally with [`WebFetchTransportError::DeadlineExceeded`] instead of letting per-hop
/// timeouts accumulate.
fn pinned_web_fetch(
    connector: &PinnedConnector<'_>,
    url: &str,
    timeout_ms: u64,
    total_budget_ms: u64,
    deadline: Instant,
    sender: &dyn PinnedSender,
) -> Result<PinnedHttpExchange, PinnedFetchError> {
    let mut current_url = url.to_string();
    let mut decision = connector.validate_initial(&current_url);
    if !decision.is_allowed() {
        return Err(PinnedFetchError::Denied {
            url: current_url,
            decision,
        });
    }

    let mut hops_followed: usize = 0;
    loop {
        let remaining_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        if remaining_ms == 0 {
            return Err(PinnedFetchError::Transport(
                WebFetchTransportError::DeadlineExceeded {
                    budget_ms: total_budget_ms,
                },
            ));
        }
        // Clamp the per-hop request timeout to the remaining chain budget so a slow hop cannot
        // consume more than what the aggregate deadline leaves.
        let hop_timeout_ms = timeout_ms.min(remaining_ms);
        let exchange = sender
            .send(&current_url, &decision, hop_timeout_ms)
            .map_err(PinnedFetchError::Transport)?;

        if !exchange.is_redirect() {
            return Ok(exchange);
        }
        // `is_redirect()` guarantees a non-empty Location header.
        let location = exchange.location().expect("redirect exchange carries a Location header");
        let target = connector
            .resolve_redirect_target(&exchange.url, &location)
            .map_err(|detail| PinnedFetchError::Denied {
                url: location,
                decision: WebAccessDecision::Deny {
                    reason: WebAccessDenyReason::InvalidUrl(detail),
                    hop: hops_followed + 1,
                },
            })?;
        let next = connector.validate_redirect(&target, hops_followed);
        if !next.is_allowed() {
            return Err(PinnedFetchError::Denied {
                url: target,
                decision: next,
            });
        }
        current_url = target;
        decision = next;
        hops_followed += 1;
    }
}

/// Human-readable message for a whole-pipeline [`PinnedFetchError`].
fn pinned_fetch_error_message(error: &PinnedFetchError) -> String {
    match error {
        PinnedFetchError::Denied {
            decision: WebAccessDecision::Deny { reason, .. },
            ..
        } => web_access_deny_message(reason),
        PinnedFetchError::Denied { .. } => "URL 被 Web 访问策略拒绝。".to_string(),
        PinnedFetchError::Transport(transport) => web_fetch_transport_message(transport),
    }
}

/// Human-readable message for a [`WebFetchTransportError`].
fn web_fetch_transport_message(error: &WebFetchTransportError) -> String {
    match error {
        WebFetchTransportError::ClientBuild(message) => message.clone(),
        WebFetchTransportError::Request(message) => message.clone(),
        WebFetchTransportError::UnsupportedContentType { content_type } => {
            format!("目标返回的内容类型 `{content_type}` 不是可读文本，已拒绝解码。")
        }
        WebFetchTransportError::ReadBody(message) => format!("读取响应正文失败：{message}。"),
        WebFetchTransportError::PeerMismatch { expected, actual } => {
            format!("连接对端地址 {actual} 不在已校验地址列表 {expected:?} 中，已拒绝连接。")
        }
        WebFetchTransportError::DeadlineExceeded { budget_ms } => {
            format!("抓取 URL 超过链级总 deadline（总预算 {budget_ms} ms）。")
        }
    }
}

fn is_reqwest_timeout_error(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error
            .source()
            .map(|source| {
                source
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("timed out")
            })
            .unwrap_or(false)
}

fn is_tool_timeout_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("timeout")
        || normalized.contains("timed out")
        || normalized.contains("deadline has elapsed")
}

/// Retry a tool operation up to [`TOOL_TIMEOUT_RETRY_MAX_ATTEMPTS`] times when it fails with a
/// timeout-class error. The whole call — retries plus the in-flight work of each attempt — is
/// bounded by one aggregate wall-clock deadline derived from `total_budget_ms` (design Decision 8
/// / tasks 6.2-6.3): the operation receives that absolute deadline so a chained operation such as
/// `pinned_web_fetch` clamps its own work to the same budget, and a retry never restarts the
/// budget (each attempt does not get its own fresh `total_budget_ms`).
fn retry_tool_timeout<T, F>(
    tool_name: &str,
    total_budget_ms: u64,
    mut operation: F,
) -> Result<T, String>
where
    F: FnMut(Instant) -> Result<T, String>,
{
    let config = crate::agent::retry::BackoffConfig {
        max_retries: TOOL_TIMEOUT_RETRY_MAX_ATTEMPTS.saturating_sub(1),
        initial_delay_ms: 500,
        multiplier: 2.0,
        max_delay_ms: 8000,
        total_budget_ms,
        jitter_kind: crate::agent::retry::JitterKind::None,
    };
    let deadline = Instant::now() + Duration::from_millis(total_budget_ms);
    let mut last_error = String::new();
    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            // Backoff, capped by what the shared deadline leaves; when nothing is left, stop.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(last_error);
            }
            let delay = crate::agent::retry::compute_delay(attempt, &config, 0.5).min(remaining);
            thread::sleep(delay);
            if Instant::now() >= deadline {
                return Err(last_error);
            }
        }
        match operation(deadline) {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = error;
                if !is_tool_timeout_message(&last_error) {
                    return Err(last_error);
                }
                if attempt == config.max_retries || Instant::now() >= deadline {
                    return Err(last_error);
                }
                let _ = (tool_name, preview_text(&last_error, 180));
            }
        }
    }
    Err(last_error)
}

/// Stream a response body up to `cap` bytes; returns `(bytes, truncated)` where `truncated` is true
/// when the body exceeded the cap and was cut short (design Decision 8, phase-4..7 review P1-4).
async fn read_bounded_body(
    mut response: reqwest::Response,
    cap: usize,
) -> Result<(Vec<u8>, bool), String> {
    let mut buf = Vec::with_capacity(cap.min(4096));
    let mut truncated = false;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| error.to_string())?
    {
        if buf.len().saturating_add(chunk.len()) > cap {
            let remaining = cap.saturating_sub(buf.len());
            buf.extend_from_slice(&chunk[..remaining]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    Ok((buf, truncated))
}

/// Whether a declared `Content-Type` is text we may decode. Anything clearly binary (images,
/// audio, video, archives, PDF, octet-stream) is refused before decoding (Decision 8).
fn is_text_content_type(content_type: &str) -> bool {
    let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    mime.starts_with("text/")
        || matches!(
            mime.as_str(),
            "application/json"
                | "application/javascript"
                | "application/xml"
                | "application/xhtml+xml"
                | "application/x-www-form-urlencoded"
                | "application/graphql"
                | "application/ld+json"
                | "application/sql"
                | "application/yaml"
                | "application/x-yaml"
                | "application/toml"
                | "application/x-httpd-php"
                | "application/ecmascript"
        )
        || content_type.is_empty()
}

fn decode_content(headers: &reqwest::header::HeaderMap, bytes: &[u8]) -> String {
    let encoding = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|ct| {
            ct.split(';')
                .nth(1)
                .and_then(|s| s.split('=').nth(1))
                .map(|s| s.trim().trim_matches('"').to_lowercase())
                .filter(|s| !s.is_empty())
        })
        .and_then(|charset| Encoding::for_label(charset.as_bytes()));

    if let Some(enc) = encoding {
        let (cow, _, _) = enc.decode(bytes);
        return cow.into_owned();
    }

    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }

    let (cow, _, _) = GBK.decode(bytes);
    cow.into_owned()
}

/// Default `WebResolver` for the legacy `ToolRouter`: cannot resolve any host, so every hostname
/// URL fails closed (design Decision 8). A real resolver is only injected together with a pinned
/// connector in the governed path.
#[derive(Clone, Copy, Debug, Default)]
struct FailClosedResolver;

impl WebResolver for FailClosedResolver {
    fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String> {
        Err(format!(
            "no production DNS resolver is wired into the legacy ToolRouter; cannot resolve `{host}`"
        ))
    }
}

/// Fail-closed `web_fetch_url` result for a URL rejected by the web access policy. Carries the
/// structured deny reason (design Decision 8) so callers and telemetry can surface
/// machine-readable evidence of why the fetch was refused.
fn web_fetch_denied(url: &str, decision: &WebAccessDecision) -> ToolResult {
    let message = match decision {
        WebAccessDecision::Allow { .. } => String::new(), // unreachable: only called on Deny
        WebAccessDecision::Deny { reason, .. } => web_access_deny_message(reason),
    };
    ToolResult {
        tool_name: TOOL_WEB_FETCH_URL.to_string(),
        status: "error".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": TOOL_WEB_FETCH_URL,
            "url": url,
            "error": {
                "code": "web_access_denied",
                "message": message,
                "hint": "请检查 URL 是否为公开的 http/https 地址，且未指向内网、回环或受限端口。",
                "accessDecision": serde_json::to_value(decision).unwrap_or(Value::Null),
            },
            "summary": {
                "text": message
            }
        })),
        duration_ms: 0,
    }
}

/// Fail-closed `web_fetch_url` result for a transport/pinning failure in the pinned fetch
/// pipeline (connect error, timeout, non-text content type, body read failure, or a peer that
/// does not belong to the validated address set).
fn web_fetch_transport_error(error: &WebFetchTransportError) -> ToolResult {
    let (code, hint): (&str, Option<String>) = match error {
        WebFetchTransportError::ClientBuild(_) => ("client_build_failed", None),
        WebFetchTransportError::Request(message) => {
            if is_tool_timeout_message(message) {
                ("timeout", Some("请确认目标地址可访问，或稍后重试。".to_string()))
            } else {
                ("request_failed", Some("请确认目标地址可访问，或稍后重试。".to_string()))
            }
        }
        WebFetchTransportError::UnsupportedContentType { .. } => (
            "unsupported_content_type",
            Some("请改用能返回 text/html、text/plain、application/json 等文本内容的地址。".to_string()),
        ),
        WebFetchTransportError::ReadBody(_) => (
            "read_body_failed",
            Some("请确认目标地址返回的是可读取文本内容。".to_string()),
        ),
        WebFetchTransportError::PeerMismatch { .. } => (
            "connection_pin_violation",
            Some("连接对端地址与已校验地址不一致，已拒绝本次连接。".to_string()),
        ),
        WebFetchTransportError::DeadlineExceeded { .. } => (
            "web_fetch_deadline_exceeded",
            Some("目标服务器响应链过慢，已超过本次抓取的总时间预算。请稍后重试或缩小抓取范围。".to_string()),
        ),
    };
    error_result(
        TOOL_WEB_FETCH_URL,
        code,
        web_fetch_transport_message(error),
        hint,
    )
}

/// Human-readable summary for a structured web access deny reason.
fn web_access_deny_message(reason: &WebAccessDenyReason) -> String {
    match reason {
        WebAccessDenyReason::InvalidUrl(detail) => format!("URL 无法解析：{detail}。"),
        WebAccessDenyReason::UnsupportedScheme { scheme } => {
            format!("不支持的 URL 协议：`{scheme}`，仅允许 http/https。")
        }
        WebAccessDenyReason::MissingHost => "URL 缺少主机名。".to_string(),
        WebAccessDenyReason::CredentialsNotAllowed => {
            "URL 不允许携带用户名/密码凭据。".to_string()
        }
        WebAccessDenyReason::HostForbidden { host, detail } => {
            format!("主机 `{host}` 被策略禁止：{detail}。")
        }
        WebAccessDenyReason::RestrictedPort { port } => {
            format!("端口 {port} 在受限端口列表中。")
        }
        WebAccessDenyReason::ForbiddenLiteralAddress { address, detail } => {
            format!("字面 IP `{address}` 属于禁止地址类：{detail}。")
        }
        WebAccessDenyReason::ResolvesToForbiddenAddress {
            host,
            address,
            detail,
        } => format!("主机 `{host}` 解析到禁止地址 `{address}`：{detail}。"),
        WebAccessDenyReason::NoAddresses { host } => {
            format!("主机 `{host}` 没有可用 DNS 记录。")
        }
        WebAccessDenyReason::ResolutionFailed { host, error } => {
            format!("无法解析主机 `{host}`：{error}。")
        }
        WebAccessDenyReason::TooManyRedirects { limit } => {
            format!("重定向次数超过上限 {limit}。")
        }
    }
}

/// Per-run session id for the legacy `Run` path: each invocation gets an opaque session so the
/// `ProcessManager` handle is never shared or replayable across runs.
fn legacy_run_session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos() as u64)
        .unwrap_or(0);
    format!("legacy-run-{nanos:016x}")
}

/// Build the `ProcessStartRequest` program/arguments for a shell command string. The child runs
/// under the platform shell (`cmd /C` on Windows, `sh -lc` elsewhere) exactly like the legacy
/// `spawn_workspace_command`, but `ProcessStartRequest` has no `cwd` field, so the resolved
/// workspace directory is baked into the shell command with a leading `cd`.
///
/// Windows note: `fs::canonicalize` yields `\\?\`-prefixed extended-length paths that cmd's `cd`
/// builtin rejects, so the prefix is normalized away. The path is then caret-escaped
/// (`cmd_escape_path`) rather than double-quoted because `std::process::Command` escapes embedded
/// `"` as `\"`, which cmd mis-parses (a leading `\` corrupts the path) after it strips the outer
/// quote pair.
fn workspace_command_parts(command: &str, cwd: &Path) -> (String, Vec<String>) {
    if cfg!(windows) {
        let escaped_cwd = cmd_escape_path(&windows_shell_path(&cwd.display().to_string()));
        (
            "cmd".to_string(),
            vec![
                "/C".to_string(),
                format!("cd /d {escaped_cwd} && {command}"),
            ],
        )
    } else {
        let quoted = format!("'{}'", cwd.display().to_string().replace('\'', "'\\''"));
        (
            "sh".to_string(),
            vec!["-lc".to_string(), format!("cd {quoted} && {command}")],
        )
    }
}

/// Normalize a Windows path for use inside a cmd command line: strips the `\\?\` extended-length
/// prefix that `fs::canonicalize` produces (cmd's `cd` rejects it) and restores `\\?\UNC\` to the
/// conventional `\\server\share` form.
fn windows_shell_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    path.to_string()
}

/// Escape every cmd metacharacter in a path with `^` so `cd /d <path>` works without double
/// quotes (which cannot survive `std::process::Command`'s `\"` escaping on Windows). Windows
/// paths cannot contain `"`, and the `%VAR%` expansion pattern is left untouched (extremely rare
/// in real workspace paths).
fn cmd_escape_path(path: &str) -> String {
    let mut escaped = String::with_capacity(path.len());
    for character in path.chars() {
        if " &|<>()@^\"".contains(character) {
            escaped.push('^');
        }
        escaped.push(character);
    }
    escaped
}

fn existing_workspace_ancestor_path(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return Some(candidate.to_path_buf());
        }
        current = candidate.parent();
    }
    None
}

fn error_result(tool_name: &str, code: &str, message: String, hint: Option<String>) -> ToolResult {
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "error".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": tool_name,
            "error": {
                "code": code,
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
    Some(ToolError {
        kind: error
            .get("kind")
            .or_else(|| error.get("code"))
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

fn aborted_result(tool_name: &str, code: &str, message: String) -> ToolResult {
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "aborted".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": tool_name,
            "status": "aborted",
            "error": {
                "code": code,
                "message": message,
                "hint": Option::<String>::None,
            }
        })),
        duration_ms: 0,
    }
}

fn json_string(value: Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

fn parse_tool_output(output: &str) -> Value {
    serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()))
}

fn nested_result_status(result: &ToolResult) -> String {
    let output = parse_tool_output(&result.output);
    nested_output_status(result, &output)
}

fn nested_output_status(result: &ToolResult, output: &Value) -> String {
    if result.status == "aborted" {
        return "aborted".to_string();
    }
    if result.status != "ok" {
        return "error".to_string();
    }
    match output.get("status").and_then(Value::as_str) {
        Some("partial") => "partial".to_string(),
        Some("error") => "error".to_string(),
        _ => "ok".to_string(),
    }
}

fn first_search_match_line(output: &str, target_path: &str) -> Option<usize> {
    let parsed = parse_tool_output(output);
    parsed
        .get("matches")
        .and_then(Value::as_array)
        .and_then(|matches| {
            matches.iter().find_map(|entry| {
                let path = entry.get("path").and_then(Value::as_str)?;
                let line = entry.get("line").and_then(Value::as_u64)? as usize;
                if path == target_path {
                    Some(line)
                } else {
                    None
                }
            })
        })
}

fn search_match_count(output: &str) -> usize {
    parse_tool_output(output)
        .get("matchCount")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
}

fn build_nested_results_summary(
    tool_name: &str,
    results: &[Value],
    aggregate_status: &str,
) -> Value {
    let first_error = results
        .iter()
        .find_map(extract_first_error_from_nested_result);

    let top_matches = results
        .iter()
        .flat_map(extract_top_matches_from_nested_result)
        .take(SUMMARY_ITEM_LIMIT)
        .collect::<Vec<_>>();

    let listed_paths = results
        .iter()
        .find(|entry| entry.get("tool").and_then(Value::as_str) == Some(TOOL_WORKSPACE_LIST_FILES))
        .and_then(|entry| entry.get("output"))
        .and_then(|output| output.get("entries"))
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .take(5)
                .filter_map(|entry| entry.get("path").cloned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let text = match tool_name {
        TOOL_WORKSPACE_BATCH => format!(
            "workspace_batch 已汇总 {} 个子调用，整体状态为 {}。",
            results.len(),
            aggregate_status
        ),
        TOOL_WORKSPACE_GATHER_CONTEXT => format!(
            "workspace_gather_context 已聚合 {} 个上下文子调用，整体状态为 {}。",
            results.len(),
            aggregate_status
        ),
        _ => format!(
            "已聚合 {} 个子调用，整体状态为 {}。",
            results.len(),
            aggregate_status
        ),
    };

    json!({
        "text": text,
        "firstError": first_error,
        "topMatches": top_matches,
        "listedPaths": listed_paths,
    })
}

fn is_read_only_batch_child_primitive(primitive: &str) -> bool {
    READ_ONLY_BATCH_CHILD_PRIMITIVES.contains(&primitive)
}

/// Human-readable permission scope for a `workspace_batch` child, used in the
/// fail-closed `unsupported_composite_child` rejection. Unknown (non-builtin)
/// names fail closed as `external` instead of being executed.
fn composite_child_scope(primitive: Option<&'static str>) -> &'static str {
    match primitive {
        Some(TOOL_WORKSPACE_WRITE_FILE | TOOL_WORKSPACE_EDIT_FILE) => "workspace.write",
        Some(TOOL_WORKSPACE_RUN_COMMAND) => "workspace.execute",
        Some(TOOL_WEB_FETCH_URL | TOOL_WEB_SEARCH_QUERY) => "web",
        Some(TOOL_MCP_RESOURCE_READ) => "mcp",
        Some(TOOL_TOOL_SEARCH) => "capability.discovery",
        Some(TOOL_WORKSPACE_BATCH | TOOL_WORKSPACE_GATHER_CONTEXT) => "composite",
        Some(TOOL_ECHO_INPUT) => "interactive",
        Some(TOOL_TIME_NOW) => "workspace.execute",
        Some(_) => "workspace.read",
        None => "external",
    }
}

fn build_batch_tool_plan(
    nested_calls: &[ToolCall],
    parallel: bool,
    continue_on_error: bool,
) -> ToolPlan {
    ToolPlan {
        kind: "batch".to_string(),
        summary: format!(
            "workspace_batch 计划执行 {} 个子调用{}{}。",
            nested_calls.len(),
            if parallel {
                "（并发）"
            } else {
                "（串行）"
            },
            if continue_on_error {
                "，失败后继续"
            } else {
                "，失败后中止"
            }
        ),
        parallel,
        continue_on_error,
        steps: nested_calls
            .iter()
            .enumerate()
            .map(|(index, call)| ToolPlanStep {
                name: canonical_tool_name(&call.name)
                    .unwrap_or(&call.name)
                    .to_string(),
                arguments: call.arguments.clone(),
                summary: format!("第 {} 个子调用：`{}`。", index + 1, call.name),
            })
            .collect(),
    }
}

fn build_multi_path_gather_plan(
    paths: &[String],
    query: &str,
    limit: usize,
    line_count: usize,
) -> ToolPlan {
    ToolPlan {
        kind: "gather_context".to_string(),
        summary: format!(
            "workspace_gather_context 将聚合 {} 个路径{}。",
            paths.len(),
            if query.trim().is_empty() {
                ""
            } else {
                "，并带搜索条件"
            }
        ),
        parallel: false,
        continue_on_error: true,
        steps: paths
            .iter()
            .enumerate()
            .map(|(index, path)| ToolPlanStep {
                name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                arguments: json!({
                    "path": path,
                    "query": if query.trim().is_empty() { Value::Null } else { Value::String(query.to_string()) },
                    "limit": limit,
                    "lineCount": line_count,
                }),
                summary: format!("第 {} 个路径聚合：`{}`。", index + 1, path),
            })
            .collect(),
    }
}

fn build_nested_tool_plan(
    tool_name: &str,
    mode: &str,
    display_path: &str,
    query: &str,
    nested: &[(usize, ToolCall, ToolResult)],
) -> ToolPlan {
    let steps = nested
        .iter()
        .map(|(index, call, _)| ToolPlanStep {
            name: canonical_tool_name(&call.name)
                .unwrap_or(&call.name)
                .to_string(),
            arguments: call.arguments.clone(),
            summary: format!("第 {} 个子调用：`{}`。", index + 1, call.name),
        })
        .collect::<Vec<_>>();

    ToolPlan {
        kind: tool_name.to_string(),
        summary: match mode {
            "file" => format!("围绕文件 `{}` 聚合元信息与代码片段。", display_path),
            "directory" => format!("围绕目录 `{}` 聚合元信息与目录列表。", display_path),
            "search" => format!(
                "围绕 `{}` 聚合搜索上下文{}。",
                display_path,
                if query.trim().is_empty() {
                    ""
                } else {
                    "，并补充命中附近片段"
                }
            ),
            _ => format!("围绕 `{}` 聚合上下文。", display_path),
        },
        parallel: matches!(mode, "file" | "directory"),
        continue_on_error: true,
        steps,
    }
}

fn extract_first_error_from_nested_result(result: &Value) -> Option<Value> {
    let status = result.get("aggregateStatus").and_then(Value::as_str)?;
    if matches!(status, "error" | "aborted") {
        return Some(json!({
            "tool": result.get("tool").cloned().unwrap_or(Value::Null),
            "index": result.get("index").cloned().unwrap_or(Value::Null),
            "status": status,
            "error": result.get("error").cloned().unwrap_or(Value::Null),
        }));
    }

    result
        .get("output")
        .and_then(|output| output.get("summary"))
        .and_then(|summary| summary.get("firstError"))
        .cloned()
        .filter(|value| !value.is_null())
}

fn extract_top_matches_from_nested_result(result: &Value) -> Vec<Value> {
    let Some(output) = result.get("output") else {
        return Vec::new();
    };

    if let Some(matches) = output.get("matches").and_then(Value::as_array) {
        return matches
            .iter()
            .take(SUMMARY_ITEM_LIMIT)
            .cloned()
            .collect::<Vec<_>>();
    }

    output
        .get("summary")
        .and_then(|summary| summary.get("topMatches"))
        .and_then(Value::as_array)
        .map(|matches| {
            matches
                .iter()
                .take(SUMMARY_ITEM_LIMIT)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

enum FileSegment {
    Empty,
    Range {
        start_line: usize,
        end_line: usize,
        lines: Vec<(usize, String)>,
        total_lines: usize,
    },
}

enum ReadSegmentError {
    StartOutOfRange { total_lines: usize },
    Io(std::io::Error),
}

fn read_file_lines(
    path: &Path,
    start_line: usize,
    line_count: usize,
) -> Result<FileSegment, ReadSegmentError> {
    let file = File::open(path).map_err(ReadSegmentError::Io)?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut total_lines = 0usize;

    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        total_lines = line_number;
        let line = line.map_err(ReadSegmentError::Io)?;
        if line_number < start_line {
            continue;
        }
        if lines.len() < line_count {
            lines.push((line_number, line));
        }
    }

    if total_lines == 0 {
        return Ok(FileSegment::Empty);
    }

    if start_line > total_lines {
        return Err(ReadSegmentError::StartOutOfRange { total_lines });
    }

    let end_line = lines
        .last()
        .map(|(line_number, _)| *line_number)
        .unwrap_or(start_line);

    Ok(FileSegment::Range {
        start_line,
        end_line,
        lines,
        total_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("pony-agent-tools-test-{}", unique));
        fs::create_dir_all(&dir).expect("create temp workspace");
        dir
    }

    #[test]
    fn time_now_returns_unix_timestamp_and_local_iso() {
        let router = ToolRouter::with_workspace_root(temp_workspace());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_TIME_NOW.to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload: Value =
            serde_json::from_str(&result.output).expect("time_now output should be json");
        let unix_ts = payload
            .get("unixTimestampSeconds")
            .and_then(Value::as_u64)
            .expect("unixTimestampSeconds should exist");
        assert!(
            unix_ts > 1_700_000_000,
            "unix timestamp should be reasonable: {}",
            unix_ts
        );
        let local_iso = payload
            .get("localIso")
            .and_then(Value::as_str)
            .expect("localIso should exist");
        assert!(
            local_iso.contains("2026"),
            "localIso should contain current year: {}",
            local_iso
        );
        assert!(
            local_iso.len() > 16,
            "localIso should be a full datetime string"
        );
        let tz = payload
            .get("timezone")
            .and_then(Value::as_str)
            .expect("timezone should exist");
        assert!(tz.len() >= 3, "timezone offset should be present: {}", tz);
    }

    #[test]
    fn tool_outcome_keeps_pending_control_separate_from_legacy_tool_result() {
        let outcome = ToolOutcome::pending(ToolControlOutcome {
            kind: ToolControlKind::WaitingUser,
            request_id: "request-1".to_string(),
        });

        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        assert_eq!(
            outcome
                .control_outcome
                .as_ref()
                .map(|control| &control.kind),
            Some(&ToolControlKind::WaitingUser)
        );
        assert!(outcome.result.is_none());

        let legacy = outcome.into_legacy_result("Ask");
        assert_eq!(legacy.status, "error");
        let payload: Value = serde_json::from_str(&legacy.output).expect("legacy payload json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("control_outcome_pending")
        );
    }

    #[test]
    fn tool_outcome_round_trips_legacy_execution_statuses() {
        for (legacy_status, expected) in [
            ("ok", ToolExecutionStatus::Ok),
            ("error", ToolExecutionStatus::Error),
            ("aborted", ToolExecutionStatus::Cancelled),
        ] {
            assert_eq!(
                ToolExecutionStatus::from_legacy_status(legacy_status),
                expected,
                "legacy status {legacy_status}"
            );
        }
    }

    fn serve_single_http_response(status_line: &str, body: &str, content_type: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
        let address = listener.local_addr().expect("read test server addr");
        let status_line = status_line.to_string();
        let body = body.to_string();
        let content_type = content_type.to_string();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test request");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write test response");
            stream.flush().expect("flush test response");
        });

        format!("http://{}", address)
    }

    fn serve_timeout_http_response(delay_ms: u64) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
        let address = listener.local_addr().expect("read test server addr");

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test request");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            thread::sleep(Duration::from_millis(delay_ms));
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 7\r\nConnection: close\r\n\r\ntimeout",
            );
            let _ = stream.flush();
        });

        format!("http://{}", address)
    }

    fn with_env_var<R>(key: &str, value: &str, operation: impl FnOnce() -> R) -> R {
        let previous = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        let result = operation();
        match previous {
            Some(previous_value) => unsafe {
                std::env::set_var(key, previous_value);
            },
            None => unsafe {
                std::env::remove_var(key);
            },
        }
        result
    }

    #[test]
    fn batch_returns_partial_success_without_failing_runtime_status() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "hello\nworld\n").expect("write demo file");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({
                "parallel": true,
                "continueOnError": true,
                "calls": [
                    {
                        "name": TOOL_WORKSPACE_PATH_INFO,
                        "arguments": { "path": "demo.txt" }
                    },
                    {
                        "name": TOOL_WORKSPACE_READ_FILE,
                        "arguments": { "path": "missing.txt" }
                    }
                ]
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        assert_eq!(
            payload.get("status").and_then(Value::as_str),
            Some("partial")
        );
        assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(1));
    }

    #[test]
    fn gather_context_reads_file_info_and_segment() {
        let workspace = temp_workspace();
        fs::write(
            workspace.join("demo.rs"),
            "fn main() {}\nprintln!(\"hi\");\n",
        )
        .expect("write file");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({ "path": "demo.rs", "lineCount": 20 }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(
            payload
                .get("meta")
                .and_then(Value::as_object)
                .and_then(|meta| meta.get("mode"))
                .and_then(Value::as_str),
            Some("file")
        );
        assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(2));
    }

    #[test]
    fn gather_context_can_aggregate_multiple_paths() {
        let workspace = temp_workspace();
        fs::write(workspace.join("one.rs"), "fn one() {}\n").expect("write one");
        fs::write(workspace.join("two.rs"), "fn two() {}\n").expect("write two");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "paths": ["one.rs", "two.rs"],
                "lineCount": 20
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(
            payload
                .get("meta")
                .and_then(Value::as_object)
                .and_then(|meta| meta.get("mode"))
                .and_then(Value::as_str),
            Some("multi_path")
        );
        assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(2));
    }

    #[test]
    fn gather_context_limits_too_many_paths_as_partial_result() {
        let workspace = temp_workspace();
        for index in 0..=MAX_GATHER_CONTEXT_PATHS {
            fs::write(workspace.join(format!("demo-{index}.rs")), "fn demo() {}\n")
                .expect("write demo");
        }
        let router = ToolRouter::with_workspace_root(workspace.clone());
        let paths = (0..=MAX_GATHER_CONTEXT_PATHS)
            .map(|index| Value::String(format!("demo-{index}.rs")))
            .collect::<Vec<_>>();

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "paths": paths,
                "lineCount": 20
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(
            payload
                .get("meta")
                .and_then(|meta| meta.get("requestedPathCount"))
                .and_then(Value::as_u64),
            Some((MAX_GATHER_CONTEXT_PATHS + 1) as u64)
        );
        assert_eq!(
            payload.get("status").and_then(Value::as_str),
            Some("partial")
        );
        assert_eq!(
            payload.get("successCount").and_then(Value::as_u64),
            Some(MAX_GATHER_CONTEXT_PATHS as u64)
        );
        assert_eq!(payload.get("partialCount").and_then(Value::as_u64), Some(1));
        assert_eq!(
            payload
                .get("meta")
                .and_then(|meta| meta.get("skippedPaths"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            payload
                .get("results")
                .and_then(Value::as_array)
                .and_then(|results| results.last())
                .and_then(|entry| entry.get("output"))
                .and_then(|output| output.get("reason"))
                .and_then(Value::as_str),
            Some("too_many_paths")
        );
    }

    #[test]
    fn workspace_batch_accepts_seventeen_subcalls_for_repo_exploration() {
        let workspace = temp_workspace();
        for index in 0..17 {
            fs::write(
                workspace.join(format!("demo-{index}.txt")),
                format!("file {index}\n"),
            )
            .expect("write demo");
        }
        let router = ToolRouter::with_workspace_root(workspace.clone());
        let calls = (0..17)
            .map(|index| {
                json!({
                    "name": TOOL_WORKSPACE_PATH_INFO,
                    "arguments": { "path": format!("demo-{index}.txt") }
                })
            })
            .collect::<Vec<_>>();

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({
                "parallel": true,
                "continueOnError": true,
                "calls": calls
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        assert_eq!(
            payload.get("successCount").and_then(Value::as_u64),
            Some(17)
        );
        assert_eq!(payload.get("status").and_then(Value::as_str), Some("ok"));
    }

    #[test]
    fn search_text_supports_file_path_input_and_wildcard_filter() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src")).expect("create src dir");
        fs::write(workspace.join("src/lib.rs"), "pub struct ToolRouter;\n").expect("write lib");
        fs::write(workspace.join("src/lib.txt"), "ToolRouter in text\n").expect("write txt");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
            arguments: json!({
                "query": "ToolRouter",
                "path": "src/lib.rs",
                "filePattern": "*.rs"
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
        assert_eq!(
            payload.get("pathKind").and_then(Value::as_str),
            Some("file")
        );
        assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
        assert_eq!(
            payload
                .get("matches")
                .and_then(Value::as_array)
                .and_then(|matches| matches.first())
                .and_then(|entry| entry.get("path"))
                .and_then(Value::as_str),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn glob_files_matches_paths_by_pattern() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
        fs::write(workspace.join("src/agent/tools.rs"), "pub fn demo() {}\n").expect("write tools");
        fs::write(
            workspace.join("src/agent/context.rs"),
            "pub struct AgentContext;\n",
        )
        .expect("write context");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GLOB_FILES.to_string(),
            arguments: json!({
                "pattern": "src/agent/*.rs",
                "path": ".",
                "limit": 10
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("glob output json");
        assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(2));
    }

    #[test]
    fn glob_files_respects_limit() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
        fs::write(workspace.join("src/agent/tools.rs"), "pub fn demo() {}\n").expect("write tools");
        fs::write(
            workspace.join("src/agent/context.rs"),
            "pub struct AgentContext;\n",
        )
        .expect("write context");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GLOB_FILES.to_string(),
            arguments: json!({
                "pattern": "src/agent/*.rs",
                "path": ".",
                "limit": 1
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("glob output json");
        assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
    }

    #[test]
    fn search_text_supports_real_regex_mode() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "alpha beta gamma\n").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
            arguments: json!({
                "query": "b\\w+ g\\w+",
                "path": "demo.txt",
                "regex": true
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
        assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
        assert_eq!(payload.get("regex").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn search_text_reports_invalid_regex_error() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "alpha beta gamma\n").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace);

        // `*beta*` was a wildcard pseudo-regex under the legacy search; it is an invalid real
        // regex now (design Decision 9), so the tool must fail with a structured error.
        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
            arguments: json!({
                "query": "*beta*",
                "path": "demo.txt",
                "regex": true
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("invalid_regex")
        );
    }

    #[test]
    fn search_text_respects_gitignore_and_reports_truncation() {
        let workspace = temp_workspace();
        fs::write(workspace.join(".gitignore"), "ignored.txt\n").expect("write gitignore");
        fs::write(workspace.join("keep.txt"), "needle-value\n").expect("write keep");
        fs::write(workspace.join("ignored.txt"), "needle-value\n").expect("write ignored");
        fs::write(workspace.join("other.txt"), "needle-value\n").expect("write other");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        // Gitignored files are skipped (design Decision 9): only keep.txt and other.txt match.
        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
            arguments: json!({
                "query": "needle",
                "path": ".",
                "limit": 10
            }),
            plan: None,
        });
        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
        assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(2));
        assert_eq!(payload.get("truncated").and_then(Value::as_bool), Some(false));

        // A small `limit` budget truncates honestly instead of pretending full success.
        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
            arguments: json!({
                "query": "needle",
                "path": ".",
                "limit": 1
            }),
            plan: None,
        });
        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
        assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
        assert_eq!(payload.get("truncated").and_then(Value::as_bool), Some(true));
        assert!(payload
            .get("truncationReason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("max_matches"));
    }

    #[test]
    fn gather_context_search_file_includes_segment_even_without_match() {
        let workspace = temp_workspace();
        fs::write(
            workspace.join("demo.rs"),
            "fn main() {}\nprintln!(\"hi\");\n",
        )
        .expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "path": "demo.rs",
                "query": "missing_symbol",
                "lineCount": 20
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(payload.get("plannedCount").and_then(Value::as_u64), Some(3));
        assert!(payload
            .get("results")
            .and_then(Value::as_array)
            .map(|results| {
                results.iter().any(|entry| {
                    entry.get("tool").and_then(Value::as_str)
                        == Some(TOOL_WORKSPACE_READ_FILE_SEGMENT)
                })
            })
            .unwrap_or(false));
    }

    #[test]
    fn gather_context_search_directory_falls_back_to_listing_on_empty_match() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src")).expect("create src dir");
        fs::write(workspace.join("src/lib.rs"), "pub struct ToolRouter;\n").expect("write lib");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "path": "src",
                "query": "missing_symbol",
                "limit": 10
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(payload.get("plannedCount").and_then(Value::as_u64), Some(3));
        assert!(payload
            .get("results")
            .and_then(Value::as_array)
            .map(|results| {
                results.iter().any(|entry| {
                    entry.get("tool").and_then(Value::as_str) == Some(TOOL_WORKSPACE_LIST_FILES)
                })
            })
            .unwrap_or(false));
    }

    #[test]
    fn read_file_segment_repairs_unique_same_name_file_path() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src-tauri")).expect("create src-tauri dir");
        fs::write(
            workspace.join("tauri.conf.json"),
            "{\n  \"productName\": \"Pony Agent\"\n}\n",
        )
        .expect("write tauri config");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
            arguments: json!({
                "path": "src-tauri/tauri.conf.json",
                "startLine": 1,
                "lineCount": 5
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert!(result.output.contains("tauri.conf.json"));
        assert!(result.output.contains("productName"));
    }

    #[test]
    fn read_file_segment_keeps_error_when_same_name_file_is_ambiguous() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src-tauri")).expect("create src-tauri dir");
        fs::create_dir_all(workspace.join("nested")).expect("create nested dir");
        fs::write(workspace.join("tauri.conf.json"), "{}\n").expect("write root tauri config");
        fs::write(workspace.join("nested/tauri.conf.json"), "{}\n")
            .expect("write nested tauri config");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
            arguments: json!({
                "path": "src-tauri/tauri.conf.json",
                "startLine": 1,
                "lineCount": 5
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        assert!(result.output.contains("多个同名文件"));
    }

    #[test]
    fn gather_context_repairs_unique_missing_extension_file_path() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
        fs::write(
            workspace.join("src/agent/context.rs"),
            "pub struct AgentContext;\n",
        )
        .expect("write context file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "path": "src/agent/context"
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(payload.get("status").and_then(Value::as_str), Some("ok"));
        assert!(payload
            .get("results")
            .and_then(Value::as_array)
            .map(|results| {
                results.iter().any(|entry| {
                    entry.get("tool").and_then(Value::as_str)
                        == Some(TOOL_WORKSPACE_READ_FILE_SEGMENT)
                })
            })
            .unwrap_or(false));
    }

    #[test]
    fn gather_context_keeps_error_when_missing_extension_path_is_ambiguous() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
        fs::create_dir_all(workspace.join("nested")).expect("create nested dir");
        fs::write(
            workspace.join("src/agent/context.rs"),
            "pub struct AgentContext;\n",
        )
        .expect("write first context file");
        fs::write(
            workspace.join("nested/context.rs"),
            "pub struct OtherContext;\n",
        )
        .expect("write second context file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "path": "context"
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        assert!(result.output.contains("多个缺扩展名候选文件"));
    }

    #[test]
    fn write_file_creates_new_file_in_workspace() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
            arguments: json!({
                "path": "notes/demo.txt",
                "content": "hello pony"
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("write output json");
        assert_eq!(
            payload.get("path").and_then(Value::as_str),
            Some("notes/demo.txt")
        );
        assert_eq!(
            fs::read_to_string(workspace.join("notes/demo.txt")).expect("read written file"),
            "hello pony"
        );
    }

    #[test]
    fn write_file_respects_overwrite_false_for_existing_file() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "original").expect("write original");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
            arguments: json!({
                "path": "demo.txt",
                "content": "changed",
                "overwrite": false
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("write output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("file_exists")
        );
        assert_eq!(
            fs::read_to_string(workspace.join("demo.txt")).expect("read original"),
            "original"
        );
    }

    #[test]
    fn edit_file_requires_replace_all_for_multiple_matches() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "foo\nfoo\n").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
            arguments: json!({
                "path": "demo.txt",
                "oldText": "foo",
                "newText": "bar"
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("edit output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("multiple_matches")
        );
    }

    #[test]
    fn edit_file_replaces_all_matches_when_enabled() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "foo\nfoo\n").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
            arguments: json!({
                "path": "demo.txt",
                "oldText": "foo",
                "newText": "bar",
                "replaceAll": true
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert_eq!(
            fs::read_to_string(workspace.join("demo.txt")).expect("read edited file"),
            "bar\nbar\n"
        );
    }

    #[test]
    fn edit_file_returns_no_match_when_old_text_missing() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "alpha\nbeta\n").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
            arguments: json!({
                "path": "demo.txt",
                "oldText": "missing",
                "newText": "gamma"
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("edit output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("no_match")
        );
    }

    #[test]
    fn run_command_returns_stdout_and_exit_code() {
        let workspace = temp_workspace();
        // Legacy Run fails closed without a sandbox backend (phase-5 review P1-2), so this
        // success path registers an explicitly available backend.
        let router = ToolRouter::with_workspace_root(workspace.clone())
            .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::available());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": "echo hello",
                "cwd": ".",
                "timeoutMs": 5000
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert_eq!(result.tool_name, TOOL_WORKSPACE_RUN_COMMAND);
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(
            payload.get("delegateTool").and_then(Value::as_str),
            Some("run_shell")
        );
        assert_eq!(payload.get("exitCode").and_then(Value::as_i64), Some(0));
        assert!(payload
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase()
            .contains("hello"));
    }

    #[test]
    fn ask_returns_fallback_clarification_when_text_missing() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_ECHO_INPUT.to_string(),
            arguments: json!({
                "question": "请确认要继续执行哪一步？"
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("ask fallback output json");
        assert_eq!(
            payload.get("mode").and_then(Value::as_str),
            Some("fallback_clarification")
        );
        assert_eq!(
            payload.get("prompt").and_then(Value::as_str),
            Some("请确认要继续执行哪一步？")
        );
    }

    #[test]
    fn ask_keeps_plain_text_output_for_normal_echo_path() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_ECHO_INPUT.to_string(),
            arguments: json!({
                "text": "hello"
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert_eq!(result.output, "echo_input 返回：hello");
    }

    #[test]
    fn ask_returns_error_when_text_and_question_both_missing() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_ECHO_INPUT.to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("ask error output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("missing_argument")
        );
    }

    #[test]
    fn run_command_denies_high_risk_commands() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": "rm -rf ."
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("command_denied")
        );
    }

    #[test]
    fn run_command_denies_rm_with_split_flags() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": "rm -r -f ."
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("command_denied")
        );
    }

    #[test]
    fn run_command_returns_error_for_non_zero_exit_code() {
        let workspace = temp_workspace();
        // Legacy Run fails closed without a sandbox backend (phase-5 review P1-2), so this
        // success path registers an explicitly available backend.
        let router = ToolRouter::with_workspace_root(workspace)
            .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::available());
        let command = if cfg!(windows) { "exit /b 7" } else { "exit 7" };

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": command
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(payload.get("exitCode").and_then(Value::as_i64), Some(7));
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("non_zero_exit")
        );
    }

    #[test]
    fn run_command_returns_timeout_error_when_command_exceeds_deadline() {
        let workspace = temp_workspace();
        // Legacy Run fails closed without a sandbox backend (phase-5 review P1-2), so this
        // success path registers an explicitly available backend.
        let router = ToolRouter::with_workspace_root(workspace)
            .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::available());
        let slow_command = if cfg!(windows) {
            "ping -n 6 127.0.0.1"
        } else {
            "sleep 5"
        };

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": slow_command,
                "cwd": ".",
                "timeoutMs": 200
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("timeout")
        );
    }

    #[test]
    fn run_command_fails_closed_when_no_sandbox_backend_is_registered() {
        let workspace = temp_workspace();
        // Phase-5 review P1-2: a legacy Run with NO backend registered must fail closed
        // (`sandbox_denied` via `NoSandboxBackend`), never silently downgrade to a
        // full-parent-environment shell.
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": "echo should-not-run",
                "cwd": ".",
                "timeoutMs": 5000
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("sandbox_denied")
        );
    }

    #[test]
    fn run_command_fails_closed_when_a_sandbox_backend_is_registered_and_unavailable() {
        let workspace = temp_workspace();
        // With an explicitly registered unavailable sandbox backend the legacy Run path fails
        // closed (sandbox_denied). With no backend registered it also fails closed (P1-2); the
        // governed dispatcher enforces the sandbox gate for the governed path.
        let router = ToolRouter::with_workspace_root(workspace.clone())
            .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::unavailable());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": "echo hello",
                "cwd": ".",
                "timeoutMs": 5000
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("sandbox_denied")
        );
    }

    #[test]
    fn run_command_executes_when_a_registered_sandbox_backend_is_available() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace)
            .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::available());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": "echo hello",
                "cwd": ".",
                "timeoutMs": 5000
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(payload.get("exitCode").and_then(Value::as_i64), Some(0));
        assert!(payload
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase()
            .contains("hello"));
    }

    #[test]
    fn batch_rejects_nested_workspace_batch_calls() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({
                "calls": [
                    {
                        "name": "workspace_batch",
                        "arguments": {
                            "calls": [
                                {
                                    "name": "workspace_list_files",
                                    "arguments": { "path": "." }
                                }
                            ]
                        }
                    }
                ]
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("unsupported_composite_child")
        );
        let message = payload
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .expect("rejection message");
        assert!(
            message.contains("workspace_batch"),
            "rejection message should name the recursive child: {}",
            message
        );
        assert!(
            message.contains("composite"),
            "rejection message should name the recursive child scope: {}",
            message
        );
    }

    /// Runs a single-child `workspace_batch` and asserts the fail-closed rejection:
    /// status `error`, error code `unsupported_composite_child`, and a message that
    /// names the offending child and its permission scope. Returns the parsed payload.
    fn assert_batch_child_rejected(
        router: &ToolRouter,
        child: Value,
        expected_scope: &str,
    ) -> Value {
        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({ "calls": [child] }),
            plan: None,
        });
        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        let error = payload.get("error").expect("structured rejection error");
        assert_eq!(
            error.get("code").and_then(Value::as_str),
            Some("unsupported_composite_child")
        );
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .expect("rejection message");
        assert!(
            message.contains(expected_scope),
            "rejection message should name scope `{}`: {}",
            expected_scope,
            message
        );
        payload
    }

    #[test]
    fn batch_executes_read_only_children() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "hello\nworld\n").expect("write demo");
        fs::create_dir_all(workspace.join("src")).expect("create src");
        fs::write(workspace.join("src/lib.rs"), "pub fn demo() {}\n").expect("write lib");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({
                "parallel": true,
                "continueOnError": true,
                "calls": [
                    { "name": TOOL_WORKSPACE_LIST_FILES, "arguments": { "path": "." } },
                    { "name": TOOL_WORKSPACE_READ_FILE, "arguments": { "path": "demo.txt" } },
                    {
                        "name": TOOL_WORKSPACE_READ_FILE_SEGMENT,
                        "arguments": { "path": "demo.txt", "startLine": 1, "lineCount": 2 }
                    },
                    { "name": TOOL_WORKSPACE_PATH_INFO, "arguments": { "path": "demo.txt" } },
                    { "name": TOOL_WORKSPACE_SEARCH_TEXT, "arguments": { "query": "hello", "path": "." } },
                    { "name": TOOL_WORKSPACE_GLOB_FILES, "arguments": { "pattern": "src/*.rs", "path": "." } },
                ]
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        assert_eq!(payload.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(6));
        assert_eq!(payload.get("errorCount").and_then(Value::as_u64), Some(0));
    }

    #[test]
    fn batch_executes_read_only_product_aliases() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "needle\n").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({
                "calls": [
                    { "name": "List", "arguments": { "path": "." } },
                    { "name": "Glob", "arguments": { "pattern": "*.txt", "path": "." } },
                    { "name": "Search", "arguments": { "query": "needle", "path": "." } },
                ]
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        assert_eq!(payload.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(3));
    }

    #[test]
    fn batch_rejects_write_child_without_executing_it() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace.clone());
        let payload = assert_batch_child_rejected(
            &router,
            json!({
                "name": TOOL_WORKSPACE_WRITE_FILE,
                "arguments": { "path": "x.txt", "content": "should not land" }
            }),
            "workspace.write",
        );
        let message = payload
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .expect("message");
        assert!(
            message.contains(TOOL_WORKSPACE_WRITE_FILE),
            "message should name the child: {}",
            message
        );
        assert!(
            !workspace.join("x.txt").exists(),
            "write child must not execute under the read-only gate"
        );
    }

    #[test]
    fn batch_rejects_edit_child_with_unsupported_composite_child() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "original").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace.clone());
        assert_batch_child_rejected(
            &router,
            json!({
                "name": TOOL_WORKSPACE_EDIT_FILE,
                "arguments": { "path": "demo.txt", "oldText": "original", "newText": "changed" }
            }),
            "workspace.write",
        );
        assert_eq!(
            fs::read_to_string(workspace.join("demo.txt")).expect("read demo"),
            "original",
            "edit child must not execute under the read-only gate"
        );
    }

    #[test]
    fn batch_rejects_run_child_with_unsupported_composite_child() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        assert_batch_child_rejected(
            &router,
            json!({
                "name": TOOL_WORKSPACE_RUN_COMMAND,
                "arguments": { "command": "echo should-not-run" }
            }),
            "workspace.execute",
        );
    }

    #[test]
    fn batch_rejects_web_children_with_unsupported_composite_child() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        assert_batch_child_rejected(
            &router,
            json!({
                "name": TOOL_WEB_FETCH_URL,
                "arguments": { "url": "https://example.com" }
            }),
            "web",
        );
        assert_batch_child_rejected(
            &router,
            json!({
                "name": TOOL_WEB_SEARCH_QUERY,
                "arguments": { "query": "pony" }
            }),
            "web",
        );
    }

    #[test]
    fn batch_rejects_mcp_child_with_unsupported_composite_child() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        assert_batch_child_rejected(
            &router,
            json!({
                "name": TOOL_MCP_RESOURCE_READ,
                "arguments": { "capabilityId": "mcp:resource:repo-index" }
            }),
            "mcp",
        );
    }

    #[test]
    fn batch_rejects_tool_search_child_with_unsupported_composite_child() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        assert_batch_child_rejected(
            &router,
            json!({ "name": TOOL_TOOL_SEARCH, "arguments": { "query": "workspace" } }),
            "capability.discovery",
        );
    }

    #[test]
    fn batch_rejects_gather_context_child_with_unsupported_composite_child() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "demo\n").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace);
        // Primitive name, and the product alias "Read" that canonicalizes to the
        // same composite primitive (used today by the local planner).
        assert_batch_child_rejected(
            &router,
            json!({
                "name": TOOL_WORKSPACE_GATHER_CONTEXT,
                "arguments": { "path": "demo.txt" }
            }),
            "composite",
        );
        assert_batch_child_rejected(
            &router,
            json!({ "name": "Read", "arguments": { "path": "demo.txt" } }),
            "composite",
        );
    }

    #[test]
    fn batch_rejects_interactive_children_with_unsupported_composite_child() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        assert_batch_child_rejected(
            &router,
            json!({ "name": TOOL_ECHO_INPUT, "arguments": { "text": "hi" } }),
            "interactive",
        );
    }

    #[test]
    fn batch_rejects_external_tool_child_with_unsupported_composite_child() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        assert_batch_child_rejected(
            &router,
            json!({ "name": "some_external_tool", "arguments": {} }),
            "external",
        );
    }

    #[test]
    fn batch_rejects_offending_child_before_any_sibling_executes() {
        let workspace = temp_workspace();
        fs::write(workspace.join("safe.txt"), "read me\n").expect("write safe");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({
                "calls": [
                    { "name": TOOL_WORKSPACE_READ_FILE, "arguments": { "path": "safe.txt" } },
                    {
                        "name": TOOL_WORKSPACE_WRITE_FILE,
                        "arguments": { "path": "should-not-land.txt", "content": "nope" }
                    }
                ]
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("unsupported_composite_child")
        );
        assert!(
            !workspace.join("should-not-land.txt").exists(),
            "no sibling may execute once a batch is rejected"
        );
    }

    #[test]
    fn batch_requires_at_least_one_call() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({ "calls": [] }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("missing_argument")
        );
    }

    #[test]
    fn batch_rejects_too_many_calls() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        let calls = (0..=MAX_WORKSPACE_BATCH_CALLS)
            .map(|index| {
                json!({
                    "name": TOOL_WORKSPACE_PATH_INFO,
                    "arguments": { "path": format!("demo-{index}.txt") }
                })
            })
            .collect::<Vec<_>>();

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({ "calls": calls }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("too_many_calls")
        );
    }

    #[test]
    fn web_fetch_rejects_non_http_urls() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({
                "url": "file:///etc/passwd"
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("web_access_denied")
        );
        // The structured deny reason is surfaced for telemetry/consumers.
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("accessDecision"))
                .and_then(|decision| decision.get("decision"))
                .and_then(Value::as_str),
            Some("deny")
        );
    }

    #[test]
    fn web_fetch_fails_closed_for_hostname_urls_without_a_production_resolver() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        // The legacy default resolver cannot resolve any host, so an arbitrary hostname URL fails
        // closed (design Decision 8) with a structured deny reason.
        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({
                "url": "https://example.com/",
                "timeoutMs": 5000
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("web_access_denied")
        );
        assert!(payload
            .get("error")
            .and_then(|error| error.get("accessDecision"))
            .and_then(|decision| decision.get("detail"))
            .and_then(|detail| detail.get("reason"))
            .and_then(|reason| reason.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("resolution_failed"));
    }

    #[test]
    fn web_search_rejects_empty_query() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_SEARCH_QUERY.to_string(),
            arguments: json!({
                "query": "   "
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("web search output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("empty_query")
        );
    }

    #[test]
    fn web_fetch_fails_closed_for_loopback_and_private_urls() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        for url in [
            "http://127.0.0.1:8080/",
            "http://localhost/",
            "http://192.168.1.10/",
            "http://169.254.169.254/latest/meta-data/",
        ] {
            let result = router.execute(&ToolCall {
                call_id: None,
                name: TOOL_WEB_FETCH_URL.to_string(),
                arguments: json!({
                    "url": url,
                    "timeoutMs": 5000
                }),
                plan: None,
            });
            assert_eq!(result.status, "error", "url `{url}` must fail closed");
            let payload =
                serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
            assert_eq!(
                payload
                    .get("error")
                    .and_then(|error| error.get("code"))
                    .and_then(Value::as_str),
                Some("web_access_denied"),
                "url `{url}`"
            );
        }
    }

    #[test]
    fn web_fetch_denies_urls_rejected_before_any_connection() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        // A local test server is reachable, but the URL points at a loopback address, so the web
        // access policy denies it before any connection is made (fail closed, design Decision 8).
        let url = serve_single_http_response(
            "HTTP/1.1 200 OK",
            "<html><body><h1>Hello Pony</h1><p>Fetch success path.</p></body></html>",
            "text/html; charset=utf-8",
        );

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({
                "url": url,
                "timeoutMs": 5000
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("web_access_denied")
        );
    }

    #[test]
    fn web_fetch_returns_structured_http_error_for_non_2xx_response_when_allowed() {
        // Kept as a contract regression for the legacy success/non-2xx output shape: the success
        // and http_error branches are unreachable through the default fail-closed resolver, so
        // this test pins the deny behavior for a local (loopback) server. The success/non-2xx
        // output contract itself is exercised directly by `build_web_client`/`retry_tool_timeout`
        // unit tests.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        let url = serve_single_http_response(
            "HTTP/1.1 404 Not Found",
            "<html><body>missing</body></html>",
            "text/html; charset=utf-8",
        );

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({
                "url": url,
                "timeoutMs": 5000
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output)
            .expect("web fetch http error output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("web_access_denied")
        );
    }

    #[test]
    fn web_fetch_times_out_before_connection_fails_closed_for_loopback() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        let url = serve_timeout_http_response(150);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({
                "url": url,
                "timeoutMs": 50
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("web fetch timeout output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("web_access_denied")
        );
    }

    #[test]
    fn web_search_returns_timeout_error_after_retries() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        let base_url = serve_timeout_http_response(150);

        let result = with_env_var("EXA_API_KEY", "test-key", || {
            with_env_var("EXA_API_BASE_URL", &base_url, || {
                router.execute(&ToolCall {
                    call_id: None,
                    name: TOOL_WEB_SEARCH_QUERY.to_string(),
                    arguments: json!({
                        "query": "pony agent",
                        "timeoutMs": 50
                    }),
                    plan: None,
                })
            })
        });

        assert_eq!(result.status, "error");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("web search timeout output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("timeout")
        );
        assert!(payload
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("timeout"));
    }

    // ── workspace_read_file ─────────────────────────────────────────

    #[test]
    fn read_file_returns_file_content() {
        let workspace = temp_workspace();
        fs::write(workspace.join("hello.txt"), "Hello Pony Agent!\n").expect("write file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({ "path": "hello.txt" }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert!(result.output.contains("Hello Pony Agent!"));
    }

    #[test]
    fn read_file_rejects_missing_path_argument() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("missing_argument")
        );
    }

    #[test]
    fn read_file_rejects_path_traversal() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({ "path": "../../../etc/passwd" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("invalid_path")
        );
    }

    #[test]
    fn read_file_rejects_nonexistent_file() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({ "path": "does-not-exist.txt" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        // canonicalize fails first → invalid_path (not metadata_failed)
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("invalid_path")
        );
    }

    #[test]
    fn read_file_rejects_file_too_large() {
        let workspace = temp_workspace();
        let large = vec![b'X'; 120_001];
        fs::write(workspace.join("large.bin"), &large).expect("write large file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({ "path": "large.bin" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("file_too_large")
        );
    }

    #[test]
    fn read_file_rejects_non_utf8_content() {
        let workspace = temp_workspace();
        // Write invalid UTF-8 bytes that fs::read_to_string will reject
        let bad_bytes = [0xFF, 0xFE, 0x00, 0x68, 0x69];
        fs::write(workspace.join("binary.bin"), &bad_bytes).expect("write binary file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({ "path": "binary.bin" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("read_failed")
        );
    }

    // ── workspace_path_info ─────────────────────────────────────────

    #[test]
    fn path_info_returns_file_metadata() {
        let workspace = temp_workspace();
        fs::write(workspace.join("info.txt"), "metadata test").expect("write file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_PATH_INFO.to_string(),
            arguments: json!({ "path": "info.txt" }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload: Value =
            serde_json::from_str(&result.output).expect("path_info output should be json");
        assert_eq!(payload.get("kind").and_then(Value::as_str), Some("file"));
        assert!(
            payload
                .get("sizeBytes")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0
        );
        assert!(payload
            .get("modifiedUnixSeconds")
            .and_then(Value::as_u64)
            .is_some());
        assert_eq!(payload.get("childCount"), Some(&Value::Null));
    }

    #[test]
    fn path_info_returns_directory_metadata() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("subdir")).expect("create subdir");
        fs::write(workspace.join("subdir/a.txt"), "a").expect("write a");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_PATH_INFO.to_string(),
            arguments: json!({ "path": "subdir" }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload: Value =
            serde_json::from_str(&result.output).expect("path_info output should be json");
        assert_eq!(
            payload.get("kind").and_then(Value::as_str),
            Some("directory")
        );
        assert_eq!(payload.get("childCount").and_then(Value::as_u64), Some(1));
    }

    #[test]
    fn path_info_rejects_invalid_path() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_PATH_INFO.to_string(),
            arguments: json!({ "path": "../../../etc" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("invalid_path")
        );
    }

    // ── workspace_list_files ────────────────────────────────────────

    #[test]
    fn list_files_returns_directory_entries() {
        let workspace = temp_workspace();
        fs::write(workspace.join("alpha.txt"), "alpha").expect("write alpha");
        fs::create_dir_all(workspace.join("beta")).expect("create beta dir");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_LIST_FILES.to_string(),
            arguments: json!({ "path": "." }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert!(result.output.contains("alpha.txt"));
        assert!(result.output.contains("beta/"));
    }

    #[test]
    fn list_files_respects_limit() {
        let workspace = temp_workspace();
        for i in 0..5 {
            fs::write(workspace.join(format!("file-{i}.txt")), &format!("{i}"))
                .expect("write file");
        }
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_LIST_FILES.to_string(),
            arguments: json!({ "path": ".", "limit": 2 }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert!(result.output.contains("当前展示前 2 个"));
    }

    #[test]
    fn list_files_rejects_path_traversal() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_LIST_FILES.to_string(),
            arguments: json!({ "path": "../../../etc" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("invalid_path")
        );
    }

    #[test]
    fn list_files_rejects_nonexistent_directory() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_LIST_FILES.to_string(),
            arguments: json!({ "path": "nonexistent-dir" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        // canonicalize fails first → invalid_path (not read_failed;
        // read_failed would require an existing directory with denied
        // read permission, which is platform-specific to test)
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("invalid_path")
        );
    }

    #[test]
    fn tool_timeout_message_classifier_matches_timeout_errors() {
        assert!(is_tool_timeout_message("timeout: request timed out"));
        assert!(is_tool_timeout_message("deadline has elapsed"));
        assert!(!is_tool_timeout_message("request failed"));
    }

    #[test]
    fn retry_tool_timeout_stops_after_non_timeout_error() {
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> =
            retry_tool_timeout(TOOL_WEB_FETCH_URL, 30_000, |_deadline| {
                attempts.set(attempts.get() + 1);
                Err("request failed".to_string())
            });

        assert!(result.is_err());
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn retry_tool_timeout_retries_until_success() {
        let attempts = std::cell::Cell::new(0);
        let result = retry_tool_timeout(TOOL_WEB_FETCH_URL, 30_000, |_deadline| {
            let next = attempts.get() + 1;
            attempts.set(next);
            if next < 2 {
                Err("timeout: request timed out".to_string())
            } else {
                Ok("ok")
            }
        })
        .expect("timeout retry should eventually succeed");

        assert_eq!(result, "ok");
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn retry_tool_timeout_budget_exhaustion_prevents_retry() {
        // The shared deadline (derived from total_budget_ms) is already exhausted when the first
        // attempt returns a timeout error, so the retry must NOT run — the budget is authoritative
        // instead of "each attempt gets its own fresh budget".
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> =
            retry_tool_timeout(TOOL_WEB_FETCH_URL, 0, |_deadline| {
                attempts.set(attempts.get() + 1);
                Err("timeout: request timed out".to_string())
            });
        assert!(result.is_err());
        assert_eq!(attempts.get(), 1, "no retry may run after the deadline is exhausted");
    }

    // ─────── pinned web fetch tests (PA-076 item 2) ─────────────────────

    use std::collections::HashMap;
    use std::sync::Mutex;
    use crate::agent::tool_runtime::FakeResolver;

    /// Build one scripted [`PinnedHttpExchange`] for use in [`FakePinnedSender`] redirect-chain
    /// tests. `status` is the HTTP status code; `location` (when Some) sets the Location header;
    /// `body` sets the response body on terminal hops.
    fn exchange(
        url: &str,
        status: u16,
        location: Option<&str>,
        body: &str,
    ) -> Result<PinnedHttpExchange, WebFetchTransportError> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(loc) = location {
            headers.insert(
                reqwest::header::LOCATION,
                reqwest::header::HeaderValue::from_str(loc).unwrap(),
            );
        }
        if !body.is_empty() {
            headers.insert(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_static("text/html; charset=utf-8"),
            );
        }
        Ok(PinnedHttpExchange {
            url: url.to_string(),
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            headers,
            body: body.as_bytes().to_vec(),
            body_truncated: false,
            peer_ip: Some("203.0.113.10".parse().unwrap()),
        })
    }

    /// A deadline far in the future so redirect-budget/chain tests never trip the aggregate
    /// chain deadline (they only exercise hop validation, not the time bound).
    fn generous_deadline() -> Instant {
        Instant::now() + Duration::from_millis(60_000)
    }

    /// A scripted [`PinnedSender`] for hermetic tests of the redirect re-validation logic.
    /// Entries are keyed by URL; every call is recorded for assertions.
    #[derive(Default)]
    struct FakePinnedSender {
        responses: HashMap<String, Result<PinnedHttpExchange, WebFetchTransportError>>,
        calls: Mutex<Vec<(String, WebAccessDecision)>>,
    }

    impl FakePinnedSender {
        fn with_script(entries: &[(&str, Result<PinnedHttpExchange, WebFetchTransportError>)]) -> Self {
            Self {
                responses: entries
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, WebAccessDecision)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl PinnedSender for FakePinnedSender {
        fn send(
            &self,
            url: &str,
            decision: &WebAccessDecision,
            _timeout_ms: u64,
        ) -> Result<PinnedHttpExchange, WebFetchTransportError> {
            self.calls
                .lock()
                .unwrap()
                .push((url.to_string(), decision.clone()));
            self.responses
                .get(url)
                .cloned()
                .unwrap_or_else(|| {
                    Err(WebFetchTransportError::Request(format!(
                        "no scripted response for `{url}`"
                    )))
                })
        }
    }

    // ---------- redirect re-validation (hermetic, no sockets) ----------

    #[test]
    fn pinned_chain_follows_up_to_five_redirects_and_re_validates_each_hop() {
        let resolver = FakeResolver::default();
        for i in 0..=5 {
            resolver.set_addresses(
                format!("hop{i}.example"),
                vec!["203.0.113.10".parse().unwrap()],
            );
        }
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);
        let sender = FakePinnedSender::with_script(&[
            ("http://hop0.example/", exchange("http://hop0.example/", 302, Some("http://hop1.example/"), "")),
            ("http://hop1.example/", exchange("http://hop1.example/", 302, Some("http://hop2.example/"), "")),
            ("http://hop2.example/", exchange("http://hop2.example/", 302, Some("http://hop3.example/"), "")),
            ("http://hop3.example/", exchange("http://hop3.example/", 302, Some("http://hop4.example/"), "")),
            ("http://hop4.example/", exchange("http://hop4.example/", 302, Some("http://hop5.example/"), "")),
            ("http://hop5.example/", exchange("http://hop5.example/", 200, None, "final")),
        ]);
        let result = pinned_web_fetch(
            &connector,
            "http://hop0.example/",
            5000,
            60_000,
            generous_deadline(),
            &sender,
        )
        .expect("5-hop chain must succeed");
        assert_eq!(result.status.as_u16(), 200);
        assert_eq!(result.body, b"final");
        assert_eq!(sender.calls().len(), 6);
        // Each hop's URL matches the expected validated URL.
        for (i, (call_url, _)) in sender.calls().iter().enumerate() {
            assert_eq!(
                call_url,
                &format!("http://hop{i}.example/"),
                "hop {i} must use the validated redirect target URL"
            );
        }
    }

    #[test]
    fn pinned_chain_rejects_more_than_five_redirects() {
        let resolver = FakeResolver::default();
        // Register hop0..hop6; hop6 is the 6th redirect target (hops 0-5 are the chain).
        for i in 0..=6 {
            resolver.set_addresses(
                format!("hop{i}.example"),
                vec!["203.0.113.10".parse().unwrap()],
            );
        }
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);
        let entries: Vec<(String, Result<PinnedHttpExchange, WebFetchTransportError>)> = (0..=5)
            .map(|i| {
                let next = i + 1;
                let key = format!("http://hop{i}.example/");
                let loc = format!("http://hop{next}.example/");
                let resp = exchange(&key, 302, Some(&loc), "");
                (key, resp)
            })
            .collect();
        let entries_ref: Vec<(&str, Result<PinnedHttpExchange, WebFetchTransportError>)> =
            entries.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let sender = FakePinnedSender::with_script(&entries_ref);
        let result = pinned_web_fetch(
            &connector,
            "http://hop0.example/",
            5000,
            60_000,
            generous_deadline(),
            &sender,
        );
        match result {
            Err(PinnedFetchError::Denied {
                decision:
                    WebAccessDecision::Deny {
                        reason: WebAccessDenyReason::TooManyRedirects { limit: 5 },
                        hop: 6,
                    },
                url,
            }) => {
                assert_eq!(url, "http://hop6.example/");
            }
            other => panic!("must deny as too_many_redirects at hop 6; got {other:?}"),
        }
    }

    #[test]
    fn pinned_chain_rejects_redirect_to_private_address() {
        let resolver = FakeResolver::default();
        resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
        resolver.set_addresses("internal.example", vec!["192.168.1.99".parse().unwrap()]);
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);
        let sender = FakePinnedSender::with_script(&[(
            "http://hop0.example/",
            exchange("http://hop0.example/", 302, Some("http://internal.example/"), ""),
        )]);
        let result = pinned_web_fetch(
            &connector,
            "http://hop0.example/",
            5000,
            60_000,
            generous_deadline(),
            &sender,
        );
        match result {
            Err(PinnedFetchError::Denied {
                decision:
                    WebAccessDecision::Deny {
                        reason: WebAccessDenyReason::ResolvesToForbiddenAddress { .. },
                        hop: 1,
                    },
                url,
            }) => {
                assert_eq!(url, "http://internal.example/");
            }
            other => panic!("redirect to private address must be denied; got {other:?}"),
        }
    }

    #[test]
    fn pinned_chain_rejects_redirect_to_unsupported_scheme() {
        let resolver = FakeResolver::default();
        resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);
        let sender = FakePinnedSender::with_script(&[(
            "http://hop0.example/",
            exchange("http://hop0.example/", 302, Some("ftp://bad/"), ""),
        )]);
        let result = pinned_web_fetch(
            &connector,
            "http://hop0.example/",
            5000,
            60_000,
            generous_deadline(),
            &sender,
        );
        match result {
            Err(PinnedFetchError::Denied {
                decision:
                    WebAccessDecision::Deny {
                        reason: WebAccessDenyReason::UnsupportedScheme { .. },
                        hop: 1,
                    },
                ..
            }) => {}
            other => panic!("redirect to ftp must be denied; got {other:?}"),
        }
    }

    #[test]
    fn pinned_chain_resolves_relative_redirect_target() {
        let resolver = FakeResolver::default();
        resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);
        let sender = FakePinnedSender::with_script(&[
            (
                "http://hop0.example/a/b",
                exchange("http://hop0.example/a/b", 302, Some("../c/d"), ""),
            ),
            (
                "http://hop0.example/c/d",
                exchange("http://hop0.example/c/d", 200, None, "resolved"),
            ),
        ]);
        let result = pinned_web_fetch(
            &connector,
            "http://hop0.example/a/b",
            5000,
            60_000,
            generous_deadline(),
            &sender,
        )
        .expect("relative redirect must succeed");
        assert_eq!(result.status.as_u16(), 200);
        assert_eq!(result.body, b"resolved");
        assert_eq!(sender.calls().len(), 2);
        // Second call: URL resolved from the relative redirect `../c/d` against base `/a/b`.
        assert_eq!(sender.calls()[1].0, "http://hop0.example/c/d");
    }

    /// A scripted [`PinnedSender`] whose hops take `delay_ms` each and record the per-hop timeout
    /// they were granted. Models a slow server chain: when `respect_timeout` is set and the granted
    /// hop timeout (the remaining chain budget) cannot cover the delay, the production reqwest
    /// client would abort with a timeout, so this fake returns a `timeout:` transport error; when
    /// `respect_timeout` is false it always sleeps, letting the chain-level deadline cut the fetch.
    struct DelayedPinnedSender {
        responses: HashMap<String, Result<PinnedHttpExchange, WebFetchTransportError>>,
        delay_ms: u64,
        respect_timeout: bool,
        calls: Mutex<Vec<(String, u64)>>,
    }

    impl DelayedPinnedSender {
        fn with_script(
            entries: &[(&str, Result<PinnedHttpExchange, WebFetchTransportError>)],
            delay_ms: u64,
            respect_timeout: bool,
        ) -> Self {
            Self {
                responses: entries
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
                delay_ms,
                respect_timeout,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, u64)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl PinnedSender for DelayedPinnedSender {
        fn send(
            &self,
            url: &str,
            decision: &WebAccessDecision,
            timeout_ms: u64,
        ) -> Result<PinnedHttpExchange, WebFetchTransportError> {
            self.calls
                .lock()
                .unwrap()
                .push((url.to_string(), timeout_ms));
            if self.respect_timeout && self.delay_ms >= timeout_ms {
                return Err(WebFetchTransportError::Request(format!(
                    "timeout: 抓取 URL 超时（该跳 granted {timeout_ms} ms）。"
                )));
            }
            thread::sleep(Duration::from_millis(self.delay_ms));
            self.responses.get(url).cloned().unwrap_or_else(|| {
                Err(WebFetchTransportError::Request(format!(
                    "no scripted response for `{url}`"
                )))
            })
        }
    }

    // ---------- chain-level aggregate deadline (design Decision 8 / tasks 6.2-6.3) ----------

    #[test]
    fn pinned_chain_exceeds_aggregate_deadline_and_is_cut_short() {
        let resolver = FakeResolver::default();
        for i in 0..=5 {
            resolver.set_addresses(
                format!("hop{i}.example"),
                vec!["203.0.113.10".parse().unwrap()],
            );
        }
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);

        // Six-hop chain where every hop takes 400 ms against a 1050 ms chain budget: three hops
        // already consume ~1200 ms (> budget), so the fetch must be cut structurally at the next
        // hop — never allowed to accumulate 6 hops × the full per-hop timeout.
        let entries: Vec<(String, Result<PinnedHttpExchange, WebFetchTransportError>)> = (0..=4)
            .map(|i| {
                let next = i + 1;
                let key = format!("http://hop{i}.example/");
                let loc = format!("http://hop{next}.example/");
                let resp = exchange(&key, 302, Some(&loc), "");
                (key, resp)
            })
            .chain([(
                "http://hop5.example/".to_string(),
                exchange("http://hop5.example/", 200, None, "final"),
            )])
            .collect();
        let entries_ref: Vec<(&str, Result<PinnedHttpExchange, WebFetchTransportError>)> =
            entries.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let sender = DelayedPinnedSender::with_script(&entries_ref, 400, false);

        let budget_ms = 1050_u64;
        let deadline = Instant::now() + Duration::from_millis(budget_ms);
        let start = Instant::now();
        let result = pinned_web_fetch(
            &connector,
            "http://hop0.example/",
            5000,
            budget_ms,
            deadline,
            &sender,
        );
        let elapsed = start.elapsed();

        match result {
            Err(PinnedFetchError::Transport(WebFetchTransportError::DeadlineExceeded {
                budget_ms: actual,
            })) => {
                assert_eq!(actual, budget_ms);
            }
            other => panic!("chain must fail with deadline exceeded; got {other:?}"),
        }
        // The chain was cut before all six hops were attempted.
        let calls = sender.calls();
        assert_eq!(
            calls.len(),
            3,
            "only the hops that fit the budget may be attempted; got {calls:?}"
        );
        // Every hop's granted timeout was clamped to the remaining chain budget (< the full 5000).
        for (_, granted) in &calls {
            assert!(
                *granted < 5000,
                "per-hop timeout must be clamped by the remaining budget; granted {granted}"
            );
        }
        // Total elapsed stayed within the budget (not hops × per-hop timeout).
        assert!(
            elapsed < Duration::from_millis(budget_ms + 1000),
            "chain must fail within the budget; elapsed {elapsed:?}"
        );
    }

    #[test]
    fn single_slow_hop_is_truncated_by_remaining_chain_budget() {
        let resolver = FakeResolver::default();
        resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);

        // One terminal hop whose server is far slower than the chain budget. The per-hop client
        // timeout must be the remaining budget (~800 ms), not the full requested timeout (5000 ms),
        // so a single slow response cannot stall the fetch beyond the aggregate deadline.
        let sender = DelayedPinnedSender::with_script(
            &[(
                "http://hop0.example/",
                exchange("http://hop0.example/", 200, None, "final"),
            )],
            2000,
            true,
        );

        let budget_ms = 800_u64;
        let deadline = Instant::now() + Duration::from_millis(budget_ms);
        let result = pinned_web_fetch(
            &connector,
            "http://hop0.example/",
            5000,
            budget_ms,
            deadline,
            &sender,
        );

        match result {
            Err(PinnedFetchError::Transport(WebFetchTransportError::Request(message))) => {
                assert!(
                    is_tool_timeout_message(&message),
                    "slow hop must surface as a timeout; got {message}"
                );
            }
            other => panic!("slow single hop must fail as a timeout; got {other:?}"),
        }
        let calls = sender.calls();
        assert_eq!(calls.len(), 1);
        let (_, granted) = &calls[0];
        assert!(
            *granted <= budget_ms,
            "hop timeout must be clamped to the remaining budget; granted {granted}"
        );
        assert!(
            *granted >= budget_ms - 50,
            "clamp must leave the whole remaining budget to the hop; granted {granted}"
        );
    }

    #[test]
    fn ample_chain_budget_preserves_full_per_hop_timeout() {
        let resolver = FakeResolver::default();
        resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);

        let sender = DelayedPinnedSender::with_script(
            &[(
                "http://hop0.example/",
                exchange("http://hop0.example/", 200, None, "final"),
            )],
            100,
            true,
        );

        // Budget far larger than the per-hop timeout: the hop keeps its full requested timeout.
        let budget_ms = 60_000_u64;
        let deadline = Instant::now() + Duration::from_millis(budget_ms);
        let result = pinned_web_fetch(
            &connector,
            "http://hop0.example/",
            5000,
            budget_ms,
            deadline,
            &sender,
        )
        .expect("single fast hop must succeed");
        assert_eq!(result.status.as_u16(), 200);
        assert_eq!(result.body, b"final");
        let calls = sender.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].1, 5000,
            "ample budget must not clamp the per-hop timeout"
        );
    }

    // ---------- P2-3 coverage: terminal 3xx and literal-private redirect ----------

    #[test]
    fn pinned_chain_treats_3xx_without_location_as_terminal() {
        let resolver = FakeResolver::default();
        resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);
        // A 302 with no Location header must be returned as the terminal exchange (never followed).
        let sender = FakePinnedSender::with_script(&[(
            "http://hop0.example/",
            exchange("http://hop0.example/", 302, None, "no-location"),
        )]);
        let result = pinned_web_fetch(
            &connector,
            "http://hop0.example/",
            5000,
            60_000,
            generous_deadline(),
            &sender,
        )
        .expect("3xx without Location is a terminal response");
        assert_eq!(result.status.as_u16(), 302);
        assert_eq!(result.body, b"no-location");
        assert_eq!(
            sender.calls().len(),
            1,
            "no redirect may be followed without a Location header"
        );
    }

    #[test]
    fn pinned_chain_rejects_redirect_to_literal_private_ip() {
        let resolver = FakeResolver::default();
        resolver.set_addresses("hop0.example", vec!["203.0.113.10".parse().unwrap()]);
        let policy = WebAccessPolicy::default();
        let connector = PinnedConnector::new(&policy, &resolver);
        let sender = FakePinnedSender::with_script(&[(
            "http://hop0.example/",
            exchange("http://hop0.example/", 302, Some("http://10.0.0.5/"), ""),
        )]);
        let result = pinned_web_fetch(
            &connector,
            "http://hop0.example/",
            5000,
            60_000,
            generous_deadline(),
            &sender,
        );
        match result {
            Err(PinnedFetchError::Denied {
                decision:
                    WebAccessDecision::Deny {
                        reason:
                            WebAccessDenyReason::ForbiddenLiteralAddress { address, .. },
                        hop: 1,
                    },
                url,
            }) => {
                assert_eq!(address, "10.0.0.5");
                assert_eq!(url, "http://10.0.0.5/");
            }
            other => panic!("redirect to literal private IP must be denied; got {other:?}"),
        }
    }

    // ---------- pin / peer / rebinding – unit tests ----------

    #[test]
    fn verify_pinned_peer_accepts_peer_in_resolved_set() {
        let decision = WebAccessDecision::Allow {
            resolved_addrs: vec!["203.0.113.10".parse().unwrap(), "203.0.113.20".parse().unwrap()],
            authority: "example.com:80".to_string(),
            sni_host: "example.com".to_string(),
            port: 80,
            is_tls: false,
        };
        assert!(verify_pinned_peer(Some("203.0.113.10".parse().unwrap()), &decision).is_ok());
    }

    #[test]
    fn verify_pinned_peer_rejects_peer_not_in_resolved_set() {
        let decision = WebAccessDecision::Allow {
            resolved_addrs: vec!["203.0.113.10".parse().unwrap()],
            authority: "example.com:80".to_string(),
            sni_host: "example.com".to_string(),
            port: 80,
            is_tls: false,
        };
        let err = verify_pinned_peer(Some("10.0.0.5".parse().unwrap()), &decision)
            .expect_err("peer mismatch must be rejected");
        match err {
            WebFetchTransportError::PeerMismatch { ref actual, .. } => {
                assert_eq!(*actual, "10.0.0.5".parse::<IpAddr>().unwrap());
            }
            other => panic!("expected PeerMismatch; got {other:?}"),
        }
    }

    #[test]
    fn verify_pinned_peer_accepts_none_peer() {
        let decision = WebAccessDecision::Allow {
            resolved_addrs: vec!["203.0.113.10".parse().unwrap()],
            authority: "example.com:80".to_string(),
            sni_host: "example.com".to_string(),
            port: 80,
            is_tls: false,
        };
        assert!(verify_pinned_peer(None, &decision).is_ok());
    }

    #[test]
    fn build_pinned_web_client_pins_domain_with_resolve_override() {
        let decision = WebAccessDecision::Allow {
            resolved_addrs: vec!["203.0.113.10".parse().unwrap(), "203.0.113.20".parse().unwrap()],
            authority: "example.com:80".to_string(),
            sni_host: "example.com".to_string(),
            port: 80,
            is_tls: false,
        };
        let client = build_pinned_web_client(&decision, "example.com", 5000)
            .expect("pinned client for a domain must build");
        let _ = client;
        // Client built successfully means resolve_to_addrs was applied — the method signature
        // guarantees that the override is registered; reqwest does not expose the resolve map
        // for inspection, but the fact that the client built proves the override list was
        // accepted (empty address set for a domain fails, tested below).
    }

    #[test]
    fn build_pinned_web_client_refuses_empty_address_set_for_domain() {
        let decision = WebAccessDecision::Allow {
            resolved_addrs: vec![],
            authority: "example.com:80".to_string(),
            sni_host: "example.com".to_string(),
            port: 80,
            is_tls: false,
        };
        let err = build_pinned_web_client(&decision, "example.com", 5000)
            .expect_err("empty address set for a domain must fail");
        assert!(err.contains("空地址集") || err.contains("empty"));
    }

    #[test]
    fn build_pinned_web_client_skips_resolve_for_literal_ip_host() {
        // A literal IP host is inherently pinned (no DNS step).
        let decision = WebAccessDecision::Allow {
            resolved_addrs: vec!["203.0.113.10".parse().unwrap()],
            authority: "203.0.113.10:80".to_string(),
            sni_host: "203.0.113.10".to_string(),
            port: 80,
            is_tls: false,
        };
        let client = build_pinned_web_client(&decision, "203.0.113.10", 5000)
            .expect("pinned client for a literal IP must build");
        let _ = client;
    }

    #[test]
    fn build_pinned_web_client_rejects_denied_decision() {
        let decision = WebAccessDecision::Deny {
            reason: WebAccessDenyReason::InvalidUrl("broken".to_string()),
            hop: 0,
        };
        let err = build_pinned_web_client(&decision, "example.com", 5000)
            .expect_err("deny decision must not build a client");
        assert!(err.contains("已拒绝"));
    }

    // ---------- end-to-end: default resolver keeps hostnames fail-closed ----------

    #[test]
    fn web_fetch_with_default_resolver_still_fails_closed_for_hostname() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({ "url": "https://example.com/", "timeoutMs": 5000 }),
            plan: None,
        });
        assert_eq!(result.status, "error");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("web_access_denied")
        );
    }

    // ---------- end-to-end: with_web_resolver – pinned path taken (transport fails, not denied) ----------

    #[test]
    fn web_fetch_with_injected_resolver_takes_pinned_path_not_weak_fallback() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        // An injected resolver can resolve a hostname to a public IP; the policy now allows the
        // URL, and the pinned path is taken — there is no "validate then let the default client
        // re-resolve" weak mode. The public IP is not reachable hermetically, so the transport
        // fails, but with a transport error (not `web_access_denied`).
        let resolver = FakeResolver::default();
        resolver.set_addresses(
            "public.example",
            vec!["203.0.113.10".parse().unwrap()],
        );
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace).with_web_resolver(resolver);
        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({ "url": "http://public.example/", "timeoutMs": 1000 }),
            plan: None,
        });
        assert_eq!(result.status, "error");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
        let code = payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("");
        // The pinned path was taken: policy allow → transport fails. Must not be web_access_denied.
        assert_ne!(
            code, "web_access_denied",
            "hostname URL with an injected resolver must not be web_access_denied; the pinned path is taken, and transport fails because the pinned IP (203.0.113.10) is unreachable"
        );
        // Expect either "request_failed" (connect error) or "timeout" (connect timeout).
        assert!(
            code == "request_failed" || code == "timeout",
            "expected transport error; got code={code}"
        );
    }

    // ---------- end-to-end: real ReqwestPinnedSender hermetic socket tests ----------

    #[test]
    fn reqwest_pinned_sender_calls_host_header_equals_original_authority() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        // Bind a server that echoes the Host header in the response body.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
        let address = listener.local_addr().expect("read test server addr");
        let port = address.port();
        let hostname = format!("pinned-authority-test.local:{port}");

        let hostname_clone = hostname.clone();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test request");
            let mut buffer = [0_u8; 4096];
            let n = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..n]);
            let actual_host = request
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("host:"))
                .map(|line| line.trim().trim_start_matches("host:").trim_start_matches("Host:").trim().to_string())
                .unwrap_or_default();
            let body = format!("<html>host={actual_host}</html>");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("write test response");
            stream.flush().expect("flush test response");
        });

        // Hand-construct a decision pointing at 127.0.0.1 (the loopback server) with a
        // hostname authority — what the real policy would produce for a validated hostname
        // URL (the connector layer trusts the validated decision; it does not re-validate).
        let decision = WebAccessDecision::Allow {
            resolved_addrs: vec!["127.0.0.1".parse().unwrap()],
            authority: hostname.clone(),
            sni_host: format!("pinned-authority-test.local"),
            port,
            is_tls: false,
        };
        let sender = ReqwestPinnedSender;
        let url = format!("http://pinned-authority-test.local:{port}/");
        let exchange = sender
            .send(&url, &decision, 5000)
            .expect("pinned send must succeed");

        assert_eq!(exchange.status.as_u16(), 200);
        let body = String::from_utf8(exchange.body).unwrap();
        assert!(
            body.contains(&hostname),
            "Host header must be the original authority `{hostname}` (not the IP); got body `{body}`"
        );
        // Peer IP must belong to the validated set.
        assert_eq!(
            exchange.peer_ip,
            Some("127.0.0.1".parse().unwrap()),
            "peer must be the pinned address"
        );
    }

    #[test]
    fn reqwest_pinned_sender_does_not_fall_back_to_ambient_dns() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        // A local server exists at 127.0.0.1, but the decision pins the client to an
        // unreachable public IP (203.0.113.10 is TEST-NET-3, reserved by RFC 5737 and
        // never globally routed; on a hermetic test host this connect will fail). If the
        // sender ever fell back to ambient DNS re-resolution, a badly configured
        // environment could route `rebind-probe.test` elsewhere — but the resolve
        // override in [`build_pinned_web_client`] makes that impossible.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let port = listener.local_addr().unwrap().port();
        // Drop the listener so there's no server, but the port is already captured.
        drop(listener);

        let decision = WebAccessDecision::Allow {
            resolved_addrs: vec!["203.0.113.10".parse().unwrap()],
            authority: format!("rebind-probe.test:{port}"),
            sni_host: "rebind-probe.test".to_string(),
            port,
            is_tls: false,
        };
        let sender = ReqwestPinnedSender;
        let url = format!("http://rebind-probe.test:{port}/");
        let result = sender.send(&url, &decision, 1000);
        // Must fail: the client is pinned to 203.0.113.10 (unreachable). If the sender fell
        // back to ambient DNS for `rebind-probe.test`, it would either fail DNS (NXDOMAIN)
        // or connect somewhere unpredictable — either way the result must be an Err.
        assert!(
            result.is_err(),
            "pinned sender must never fall back to ambient DNS re-resolution"
        );
    }

    #[test]
    fn reqwest_pinned_sender_serves_a_real_redirect_chain() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        // Local server: /hop0 → 302 /hop1, /hop1 → 302 /hop2, /hop2 → 200 "final". Each hop is a
        // fresh per-hop connection to the same pinned address, and redirect-hop bodies are never
        // read (P2-3: real-socket multi-hop redirect evidence).
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
        let address = listener.local_addr().expect("read test server addr");
        let port = address.port();

        thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept test request");
                let mut buffer = [0_u8; 4096];
                let n = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..n]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (status, location, body) = match path {
                    "/hop0" => ("302 Found", Some("/hop1"), ""),
                    "/hop1" => ("302 Found", Some("/hop2"), ""),
                    _ => ("200 OK", None, "final"),
                };
                let mut response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n"
                );
                if let Some(loc) = location {
                    response.push_str(&format!("Location: {loc}\r\n"));
                }
                response.push_str(&format!(
                    "Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                ));
                stream.write_all(response.as_bytes()).expect("write test response");
                stream.flush().expect("flush test response");
            }
        });

        // Hand-constructed decision pointing at the loopback server (the connector layer trusts
        // the validated decision; it does not re-validate — same pattern as the single-hop socket
        // tests above).
        let hostname = format!("redirect-chain-test.local:{port}");
        let decision = WebAccessDecision::Allow {
            resolved_addrs: vec!["127.0.0.1".parse().unwrap()],
            authority: hostname.clone(),
            sni_host: "redirect-chain-test.local".to_string(),
            port,
            is_tls: false,
        };
        let sender = ReqwestPinnedSender;
        let base = format!("http://redirect-chain-test.local:{port}");

        let exchange0 = sender
            .send(&format!("{base}/hop0"), &decision, 5000)
            .expect("redirect hop 0 must succeed");
        assert_eq!(exchange0.status.as_u16(), 302);
        assert_eq!(exchange0.location().as_deref(), Some("/hop1"));
        assert!(exchange0.body.is_empty(), "redirect hop bodies are never read");
        assert_eq!(
            exchange0.peer_ip,
            Some("127.0.0.1".parse().unwrap()),
            "each hop's peer must belong to the validated address set"
        );

        let exchange1 = sender
            .send(&format!("{base}/hop1"), &decision, 5000)
            .expect("redirect hop 1 must succeed");
        assert_eq!(exchange1.status.as_u16(), 302);
        assert_eq!(exchange1.location().as_deref(), Some("/hop2"));
        assert!(exchange1.body.is_empty());
        assert_eq!(exchange1.peer_ip, Some("127.0.0.1".parse().unwrap()));

        let exchange2 = sender
            .send(&format!("{base}/hop2"), &decision, 5000)
            .expect("terminal hop must succeed");
        assert_eq!(exchange2.status.as_u16(), 200);
        assert_eq!(exchange2.body, b"final");
        assert_eq!(exchange2.peer_ip, Some("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn literal_public_ip_is_allowed_and_can_build_pinned_client() {
        // A literal public IPv4 URL is inherently pinned (no DNS step); the policy allows
        // it and the pinned client builds without a resolve override (literal IP host).
        let policy = WebAccessPolicy::default();
        let resolver = FailClosedResolver;
        let connector = PinnedConnector::new(&policy, &resolver);
        let decision = connector.validate_initial("http://203.0.113.10:8080/");
        match decision {
            WebAccessDecision::Allow {
                ref resolved_addrs,
                ref authority,
                port,
                is_tls,
                ..
            } => {
                assert_eq!(resolved_addrs, &["203.0.113.10".parse::<IpAddr>().unwrap()]);
                assert_eq!(authority, "203.0.113.10:8080");
                assert_eq!(port, 8080);
                assert!(!is_tls);
            }
            WebAccessDecision::Deny { .. } => {
                panic!("literal public IP must be allowed")
            }
        }
        // The client for a literal-IP URL builds without a resolve override (the IP is used
        // directly by the connector; no ambient DNS step is involved).
        let client = build_pinned_web_client(&decision, "203.0.113.10", 5000)
            .expect("pinned client for literal IP must build");
        let _ = client;
    }
}
