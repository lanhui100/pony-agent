use serde::{Deserialize, Serialize};

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
    OutOfScope,
    MalformedResponse,
    InvocationFailed,
    CapabilityNotFound,
}

impl CapabilityFailureKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SourceUnavailable => "source_unavailable",
            Self::PermissionDenied => "permission_denied",
            Self::OutOfScope => "out_of_scope",
            Self::MalformedResponse => "malformed_response",
            Self::InvocationFailed => "invocation_failed",
            Self::CapabilityNotFound => "capability_not_found",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSourceKind {
    Host,
    Mcp,
}

impl SkillSourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Mcp => "mcp",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillFailureLayer {
    SkillResolution,
    SourceUnavailable,
    PermissionDenied,
    MalformedComposition,
    UnsupportedComposition,
    UnderlyingCapabilityExecution,
}

impl SkillFailureLayer {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SkillResolution => "skill_resolution",
            Self::SourceUnavailable => "source_unavailable",
            Self::PermissionDenied => "permission_denied",
            Self::MalformedComposition => "malformed_composition",
            Self::UnsupportedComposition => "unsupported_composition",
            Self::UnderlyingCapabilityExecution => "underlying_capability_execution",
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ingress_observation: Option<SourceIngressObservation>,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSourceView {
    pub source_id: String,
    pub source_kind: SkillSourceKind,
    pub display_name: String,
    pub availability: CapabilityAvailability,
    pub transport_kind: String,
    pub server_identity: String,
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ingress_observation: Option<SourceIngressObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceIngressObservation {
    pub boundary: String,
    pub summary: String,
    pub candidate_ids: Vec<String>,
    pub observed_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDescriptor {
    pub skill_id: String,
    pub source_id: String,
    pub source_kind: SkillSourceKind,
    pub label: String,
    pub description: String,
    pub input_schema_summary: String,
    pub safety_class: String,
    pub visibility: String,
    pub observability_tags: Vec<String>,
    pub requires_approval: bool,
    pub host_mediated: bool,
    pub permission_scope: String,
    pub composed_capability_refs: Vec<String>,
    pub composed_capability_kinds: Vec<CapabilityKind>,
    pub executable_in_v1: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSourceSnapshot {
    pub source: SkillSourceView,
    pub skills: Vec<SkillDescriptor>,
}
