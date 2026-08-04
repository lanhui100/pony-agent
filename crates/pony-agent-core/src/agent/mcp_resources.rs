//! MCP resource surface (phase-7 task 7.2).
//!
//! Exposes list-resources, list-resource-templates, and read-resource as three separate
//! read-only control entries that call a source-bound [`McpTransport`]. The registry snapshot
//! is discovery metadata only: it is used to bind a `source_id`/`source_revision` pair and to
//! fail closed on unknown sources or revision mismatches, but never as trusted content.
//!
//! Transport responses are treated as **untrusted content**: every field is bounded
//! (byte/item/MIME/depth), any bound hit is surfaced as `truncated` evidence, malformed
//! payloads are rejected, and query arguments are never echoed back as resource content.

use crate::agent::tool_runtime::{
    McpResourceOperation, McpTransport, McpTransportRequest, McpTransportResponse,
};
use crate::agent::tools::{ToolDescriptorSource, ToolRegistrySnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

/// Bounds applied to untrusted MCP resource content before it reaches a model or host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpResourceLimits {
    /// Maximum resources or templates returned in one list result.
    pub max_items: usize,
    /// Maximum characters for a resource uri / uri template.
    pub max_uri_chars: usize,
    /// Maximum characters for a resource or template name / mime type.
    pub max_name_chars: usize,
    /// Maximum characters for a description.
    pub max_description_chars: usize,
    /// Maximum content items returned by a single read.
    pub max_content_items: usize,
    /// Cumulative byte budget across the content items of a single read.
    pub max_content_bytes: usize,
    /// Maximum characters for a single content text field.
    pub max_content_chars: usize,
}

impl Default for McpResourceLimits {
    fn default() -> Self {
        Self {
            max_items: 256,
            max_uri_chars: 2_048,
            max_name_chars: 256,
            max_description_chars: 512,
            max_content_items: 64,
            max_content_bytes: 1_048_576,
            max_content_chars: 200_000,
        }
    }
}

/// A discovered MCP resource (discovery metadata, never trusted content).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpResource {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// An MCP resource template. Deliberately independent from MCP prompt templates and carries
/// its source provenance so callers can bind a template back to the transport that produced it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResourceTemplate {
    pub uri_template: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub source_id: String,
    pub source_revision: String,
}

/// Bounded result of a list-resources call.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceListResult {
    pub source_id: String,
    pub source_revision: String,
    pub resources: Vec<McpResource>,
    pub truncated: bool,
}

/// Bounded result of a list-resource-templates call.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceTemplateListResult {
    pub source_id: String,
    pub source_revision: String,
    pub templates: Vec<ResourceTemplate>,
    pub truncated: bool,
}

/// One bounded piece of resource content returned by a read-resource call.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceContent {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub text: String,
}

/// Bounded result of a read-resource call.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceReadResult {
    pub source_id: String,
    pub source_revision: String,
    pub uri: String,
    pub contents: Vec<McpResourceContent>,
    pub truncated: bool,
}

/// Source-bound, read-only MCP resource surface.
#[derive(Clone)]
pub struct McpResourceSurface {
    transport: Arc<dyn McpTransport>,
    registry: ToolRegistrySnapshot,
    limits: McpResourceLimits,
}

impl McpResourceSurface {
    pub fn new(transport: Arc<dyn McpTransport>, registry: ToolRegistrySnapshot) -> Self {
        Self {
            transport,
            registry,
            limits: McpResourceLimits::default(),
        }
    }

    pub fn with_limits(mut self, limits: McpResourceLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn registry(&self) -> &ToolRegistrySnapshot {
        &self.registry
    }

    /// Replace the discovery snapshot (e.g. after a governed `replace_source`).
    pub fn set_registry(&mut self, registry: ToolRegistrySnapshot) {
        self.registry = registry;
    }

    pub fn list_resources(
        &self,
        source_id: &str,
        source_revision: &str,
    ) -> Result<McpResourceListResult, String> {
        let arguments = json!({});
        self.validate_source_binding(source_id, source_revision)?;
        let response = self.execute(source_id, source_revision, McpResourceOperation::ListResources, &arguments)?;
        self.reject_argument_echo(&response, &arguments)?;
        self.parse_resource_list(response, source_id, source_revision)
    }

    pub fn list_resource_templates(
        &self,
        source_id: &str,
        source_revision: &str,
    ) -> Result<McpResourceTemplateListResult, String> {
        let arguments = json!({});
        self.validate_source_binding(source_id, source_revision)?;
        let response = self.execute(source_id, source_revision, McpResourceOperation::ListResourceTemplates, &arguments)?;
        self.reject_argument_echo(&response, &arguments)?;
        self.parse_resource_template_list(response, source_id, source_revision)
    }

    pub fn read_resource(
        &self,
        source_id: &str,
        source_revision: &str,
        uri: &str,
    ) -> Result<McpResourceReadResult, String> {
        if uri.trim().is_empty() {
            return Err("MCP resource uri cannot be empty".to_string());
        }
        let arguments = json!({ "uri": uri });
        self.validate_source_binding(source_id, source_revision)?;
        let response = self.execute(source_id, source_revision, McpResourceOperation::ReadResource, &arguments)?;
        self.reject_argument_echo(&response, &arguments)?;
        self.parse_resource_read(response, source_id, source_revision, uri)
    }

    fn execute(
        &self,
        source_id: &str,
        source_revision: &str,
        operation: McpResourceOperation,
        arguments: &Value,
    ) -> Result<McpTransportResponse, String> {
        self.transport.execute(&McpTransportRequest {
            source_id: source_id.to_string(),
            source_revision: source_revision.to_string(),
            operation,
            arguments: arguments.clone(),
        })
    }

    /// Fail closed unless the source exists in the discovery snapshot, is an MCP source, and
    /// the requested revision matches what the snapshot currently holds for that source.
    fn validate_source_binding(&self, source_id: &str, source_revision: &str) -> Result<(), String> {
        if source_id.trim().is_empty() {
            return Err("MCP source id cannot be empty".to_string());
        }
        let mut revisions = std::collections::BTreeSet::new();
        for descriptor in &self.registry.descriptors {
            if descriptor.handler_provenance.source_id == source_id
                && descriptor.identity.source == ToolDescriptorSource::Mcp
            {
                revisions.insert(descriptor.source_revision.clone());
            }
        }
        if revisions.is_empty() {
            return Err(format!("unknown MCP source `{source_id}` in the registry snapshot"));
        }
        if revisions.len() > 1 {
            return Err(format!(
                "MCP source `{source_id}` has inconsistent revisions across descriptors"
            ));
        }
        let expected = revisions.into_iter().next().expect("revisions checked non-empty");
        if expected != source_revision {
            return Err(format!(
                "MCP source `{source_id}` revision mismatch: requested `{source_revision}`, registry holds `{expected}`"
            ));
        }
        Ok(())
    }

    /// Refuse responses that echo the query arguments back as content. A source that answers
    /// with our own request is not a real resource read and must not be trusted.
    fn reject_argument_echo(&self, response: &McpTransportResponse, arguments: &Value) -> Result<(), String> {
        if &response.content == arguments {
            return Err(self.argument_echo_error());
        }
        if let Ok(serialized) = serde_json::to_string(arguments) {
            if contains_string_equal_to(&response.content, serialized.trim()) {
                return Err(self.argument_echo_error());
            }
        }
        Ok(())
    }

    fn argument_echo_error(&self) -> String {
        "MCP resource response echoes the query arguments and is refused as untrusted".to_string()
    }

    fn parse_resource_list(
        &self,
        response: McpTransportResponse,
        source_id: &str,
        source_revision: &str,
    ) -> Result<McpResourceListResult, String> {
        let object = response.content.as_object().ok_or_else(|| {
            "MCP list-resources response content must be a JSON object".to_string()
        })?;
        let raw = object
            .get("resources")
            .and_then(Value::as_array)
            .ok_or_else(|| "MCP list-resources response must contain a `resources` array".to_string())?;
        let mut resources = Vec::new();
        let mut truncated = response.truncated;
        for item in raw {
            if resources.len() >= self.limits.max_items {
                truncated = true;
                break;
            }
            resources.push(self.parse_resource_item(item, &mut truncated)?);
        }
        Ok(McpResourceListResult {
            source_id: source_id.to_string(),
            source_revision: source_revision.to_string(),
            resources,
            truncated,
        })
    }

    fn parse_resource_item(&self, raw: &Value, truncated: &mut bool) -> Result<McpResource, String> {
        let object = raw.as_object().ok_or_else(|| "MCP resource item must be a JSON object".to_string())?;
        let uri = object
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| "MCP resource item must contain a string `uri`".to_string())?;
        Ok(McpResource {
            uri: self.truncate_string(uri, self.limits.max_uri_chars, truncated),
            name: object
                .get("name")
                .and_then(Value::as_str)
                .map(|value| self.truncate_string(value, self.limits.max_name_chars, truncated)),
            mime_type: object
                .get("mimeType")
                .and_then(Value::as_str)
                .map(|value| self.truncate_string(value, self.limits.max_name_chars, truncated)),
            description: object
                .get("description")
                .and_then(Value::as_str)
                .map(|value| self.truncate_string(value, self.limits.max_description_chars, truncated)),
        })
    }

    fn parse_resource_template_list(
        &self,
        response: McpTransportResponse,
        source_id: &str,
        source_revision: &str,
    ) -> Result<McpResourceTemplateListResult, String> {
        let object = response.content.as_object().ok_or_else(|| {
            "MCP list-resource-templates response content must be a JSON object".to_string()
        })?;
        let raw = object
            .get("resourceTemplates")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                "MCP list-resource-templates response must contain a `resourceTemplates` array"
                    .to_string()
            })?;
        let mut templates = Vec::new();
        let mut truncated = response.truncated;
        for item in raw {
            if templates.len() >= self.limits.max_items {
                truncated = true;
                break;
            }
            templates.push(self.parse_resource_template_item(item, source_id, source_revision, &mut truncated)?);
        }
        Ok(McpResourceTemplateListResult {
            source_id: source_id.to_string(),
            source_revision: source_revision.to_string(),
            templates,
            truncated,
        })
    }

    fn parse_resource_template_item(
        &self,
        raw: &Value,
        source_id: &str,
        source_revision: &str,
        truncated: &mut bool,
    ) -> Result<ResourceTemplate, String> {
        let object = raw.as_object().ok_or_else(|| "MCP resource template item must be a JSON object".to_string())?;
        let uri_template = object
            .get("uriTemplate")
            .and_then(Value::as_str)
            .ok_or_else(|| "MCP resource template item must contain a string `uriTemplate`".to_string())?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "MCP resource template item must contain a string `name`".to_string())?;
        Ok(ResourceTemplate {
            uri_template: self.truncate_string(uri_template, self.limits.max_uri_chars, truncated),
            name: self.truncate_string(name, self.limits.max_name_chars, truncated),
            description: object
                .get("description")
                .and_then(Value::as_str)
                .map(|value| self.truncate_string(value, self.limits.max_description_chars, truncated)),
            mime_type: object
                .get("mimeType")
                .and_then(Value::as_str)
                .map(|value| self.truncate_string(value, self.limits.max_name_chars, truncated)),
            source_id: source_id.to_string(),
            source_revision: source_revision.to_string(),
        })
    }

    fn parse_resource_read(
        &self,
        response: McpTransportResponse,
        source_id: &str,
        source_revision: &str,
        requested_uri: &str,
    ) -> Result<McpResourceReadResult, String> {
        let object = response.content.as_object().ok_or_else(|| {
            "MCP read-resource response content must be a JSON object".to_string()
        })?;
        let raw = object
            .get("contents")
            .and_then(Value::as_array)
            .ok_or_else(|| "MCP read-resource response must contain a `contents` array".to_string())?;
        let mut contents = Vec::new();
        let mut truncated = response.truncated;
        let mut total_bytes = 0usize;
        for item in raw {
            if contents.len() >= self.limits.max_content_items {
                truncated = true;
                break;
            }
            let content = self.parse_content_item(item, requested_uri, &mut truncated)?;
            let item_bytes =
                content.text.len() + content.uri.len() + content.mime_type.as_deref().map_or(0, str::len);
            if total_bytes + item_bytes > self.limits.max_content_bytes {
                truncated = true;
                if contents.is_empty() {
                    // Still return the single bounded item with explicit evidence rather than
                    // silently dropping the whole read.
                    contents.push(content);
                }
                break;
            }
            total_bytes += item_bytes;
            contents.push(content);
        }
        Ok(McpResourceReadResult {
            source_id: source_id.to_string(),
            source_revision: source_revision.to_string(),
            uri: requested_uri.to_string(),
            contents,
            truncated,
        })
    }

    fn parse_content_item(
        &self,
        raw: &Value,
        requested_uri: &str,
        truncated: &mut bool,
    ) -> Result<McpResourceContent, String> {
        let object = raw.as_object().ok_or_else(|| "MCP resource content item must be a JSON object".to_string())?;
        let uri = object
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| "MCP resource content item must contain a string `uri`".to_string())?;
        if uri != requested_uri {
            return Err(format!(
                "MCP resource content item uri `{uri}` does not match the requested uri `{requested_uri}`"
            ));
        }
        let text = object
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| "MCP resource content item must contain a string `text`".to_string())?;
        if text.trim() == requested_uri {
            return Err(self.argument_echo_error());
        }
        Ok(McpResourceContent {
            uri: uri.to_string(),
            mime_type: object
                .get("mimeType")
                .and_then(Value::as_str)
                .map(|value| self.truncate_string(value, self.limits.max_name_chars, truncated)),
            text: self.truncate_string(text, self.limits.max_content_chars, truncated),
        })
    }

    fn truncate_string(&self, value: &str, limit: usize, truncated: &mut bool) -> String {
        if value.chars().count() > limit {
            *truncated = true;
            value.chars().take(limit).collect()
        } else {
            value.to_string()
        }
    }
}

/// Depth-first search for a string leaf equal to `needle`. Used to detect argument echo in
/// nested untrusted content without trusting the response shape.
fn contains_string_equal_to(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.trim() == needle,
        Value::Object(map) => map.values().any(|nested| contains_string_equal_to(nested, needle)),
        Value::Array(items) => items.iter().any(|nested| contains_string_equal_to(nested, needle)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tool_runtime::FakeMcpTransport;
    use crate::agent::tools::{
        ToolDisplayMetadata, ToolExecutionPolicy, ToolHandlerProvenance, ToolIdentity, ToolKind,
        ToolPermissionDeclaration,
    };
    use serde_json::json;

    fn mcp_descriptor(source_id: &str, revision: &str) -> crate::agent::tools::ToolDescriptor {
        crate::agent::tools::ToolDescriptor {
            identity: ToolIdentity {
                descriptor_id: format!("mcp:{source_id}:resource_tool"),
                model_name: "MCPResourceTool".to_string(),
                canonical_name: "MCPResourceTool".to_string(),
                primitive_name: "mcp_resource_read".to_string(),
                source: ToolDescriptorSource::Mcp,
            },
            aliases: vec!["mcp.resource_tool".to_string()],
            description: "MCP resource tool".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
            kind: ToolKind::Read,
            exposure: crate::agent::tools::ToolExposure::Internal,
            permission_declaration: ToolPermissionDeclaration::default(),
            execution_policy: ToolExecutionPolicy::default(),
            display_metadata: ToolDisplayMetadata::default(),
            handler_provenance: ToolHandlerProvenance {
                handler_kind: "test".to_string(),
                source_id: source_id.to_string(),
            },
            source_revision: revision.to_string(),
            composed_descriptor_ids: Vec::new(),
        }
    }

    fn test_registry(source_id: &str, revision: &str) -> ToolRegistrySnapshot {
        ToolRegistrySnapshot::from_descriptors(
            "mcp-snapshot-1",
            vec![mcp_descriptor(source_id, revision)],
        )
        .expect("mcp test registry should build")
    }

    fn fake_transport() -> Arc<FakeMcpTransport> {
        Arc::new(FakeMcpTransport::default())
    }

    #[test]
    fn list_resources_returns_bounded_resources() {
        let transport = fake_transport();
        transport.set_response(
            "mcp-server-a",
            McpResourceOperation::ListResources,
            Ok(McpTransportResponse {
                content: json!({
                    "resources": [
                        { "uri": "file:///docs/readme.md", "name": "README", "mimeType": "text/markdown" },
                        { "uri": "file:///docs/guide.md", "name": "Guide", "description": "usage guide" }
                    ]
                }),
                truncated: false,
            }),
        );
        let surface = McpResourceSurface::new(transport, test_registry("mcp-server-a", "v1"));

        let result = surface
            .list_resources("mcp-server-a", "v1")
            .expect("list resources should succeed");
        assert_eq!(result.source_id, "mcp-server-a");
        assert_eq!(result.source_revision, "v1");
        assert!(!result.truncated);
        assert_eq!(result.resources.len(), 2);
        assert_eq!(result.resources[0].uri, "file:///docs/readme.md");
        assert_eq!(result.resources[0].name.as_deref(), Some("README"));
        assert_eq!(result.resources[0].mime_type.as_deref(), Some("text/markdown"));
        assert_eq!(result.resources[1].description.as_deref(), Some("usage guide"));
    }

    #[test]
    fn list_resource_templates_carries_source_provenance() {
        let transport = fake_transport();
        transport.set_response(
            "mcp-server-a",
            McpResourceOperation::ListResourceTemplates,
            Ok(McpTransportResponse {
                content: json!({
                    "resourceTemplates": [
                        { "uriTemplate": "file:///logs/{id}", "name": "Log", "mimeType": "text/plain" }
                    ]
                }),
                truncated: false,
            }),
        );
        let surface = McpResourceSurface::new(transport, test_registry("mcp-server-a", "v1"));

        let result = surface
            .list_resource_templates("mcp-server-a", "v1")
            .expect("list resource templates should succeed");
        assert_eq!(result.templates.len(), 1);
        assert_eq!(result.templates[0].uri_template, "file:///logs/{id}");
        assert_eq!(result.templates[0].source_id, "mcp-server-a");
        assert_eq!(result.templates[0].source_revision, "v1");
    }

    #[test]
    fn read_resource_returns_bounded_content() {
        let transport = fake_transport();
        transport.set_response(
            "mcp-server-a",
            McpResourceOperation::ReadResource,
            Ok(McpTransportResponse {
                content: json!({
                    "contents": [
                        { "uri": "file:///docs/readme.md", "mimeType": "text/markdown", "text": "# README" }
                    ]
                }),
                truncated: false,
            }),
        );
        let surface = McpResourceSurface::new(transport, test_registry("mcp-server-a", "v1"));

        let result = surface
            .read_resource("mcp-server-a", "v1", "file:///docs/readme.md")
            .expect("read resource should succeed");
        assert_eq!(result.uri, "file:///docs/readme.md");
        assert_eq!(result.contents.len(), 1);
        assert_eq!(result.contents[0].text, "# README");
        assert!(!result.truncated);
    }

    #[test]
    fn truncated_transport_response_is_surfaced() {
        let transport = fake_transport();
        transport.set_response(
            "mcp-server-a",
            McpResourceOperation::ListResources,
            Ok(McpTransportResponse {
                content: json!({ "resources": [{ "uri": "file:///a" }] }),
                truncated: true,
            }),
        );
        let surface = McpResourceSurface::new(transport, test_registry("mcp-server-a", "v1"));
        let result = surface
            .list_resources("mcp-server-a", "v1")
            .expect("truncated response is still a valid result");
        assert!(result.truncated);
    }

    #[test]
    fn item_cap_sets_truncated_evidence() {
        let transport = fake_transport();
        transport.set_response(
            "mcp-server-a",
            McpResourceOperation::ListResources,
            Ok(McpTransportResponse {
                content: json!({
                    "resources": [
                        { "uri": "file:///1" },
                        { "uri": "file:///2" },
                        { "uri": "file:///3" }
                    ]
                }),
                truncated: false,
            }),
        );
        let surface = McpResourceSurface::new(transport, test_registry("mcp-server-a", "v1"))
            .with_limits(McpResourceLimits {
                max_items: 2,
                ..McpResourceLimits::default()
            });
        let result = surface
            .list_resources("mcp-server-a", "v1")
            .expect("capped list should succeed");
        assert!(result.truncated);
        assert_eq!(result.resources.len(), 2);
    }

    #[test]
    fn unknown_source_fails_closed() {
        let surface = McpResourceSurface::new(fake_transport(), test_registry("mcp-server-a", "v1"));
        let error = surface
            .list_resources("missing-server", "v1")
            .expect_err("unknown source must fail closed");
        assert!(error.contains("unknown MCP source"), "{error}");
    }

    #[test]
    fn source_revision_mismatch_fails_closed() {
        let surface = McpResourceSurface::new(fake_transport(), test_registry("mcp-server-a", "v1"));
        let error = surface
            .list_resources("mcp-server-a", "v2")
            .expect_err("revision mismatch must fail closed");
        assert!(error.contains("revision mismatch"), "{error}");
    }

    #[test]
    fn builtin_source_is_not_a_resource_source() {
        let registry = ToolRegistrySnapshot::builtin().expect("builtin registry should build");
        let surface = McpResourceSurface::new(fake_transport(), registry);
        let error = surface
            .list_resources("builtin-tools", "builtin-tool-catalog-v1")
            .expect_err("builtin source has no MCP descriptors and must be refused");
        assert!(error.contains("unknown MCP source"), "{error}");
    }

    #[test]
    fn args_echo_is_rejected_as_untrusted() {
        let transport = fake_transport();
        // The transport literally answers with the query arguments object.
        transport.set_response(
            "mcp-server-a",
            McpResourceOperation::ReadResource,
            Ok(McpTransportResponse {
                content: json!({ "uri": "file:///docs/readme.md" }),
                truncated: false,
            }),
        );
        let surface = McpResourceSurface::new(transport, test_registry("mcp-server-a", "v1"));
        let error = surface
            .read_resource("mcp-server-a", "v1", "file:///docs/readme.md")
            .expect_err("a response equal to the query arguments must be refused");
        assert!(error.contains("echoes the query arguments"), "{error}");

        // A content item whose text is exactly the requested uri is also an echo.
        let transport = fake_transport();
        transport.set_response(
            "mcp-server-a",
            McpResourceOperation::ReadResource,
            Ok(McpTransportResponse {
                content: json!({
                    "contents": [
                        { "uri": "file:///docs/readme.md", "text": "file:///docs/readme.md" }
                    ]
                }),
                truncated: false,
            }),
        );
        let surface = McpResourceSurface::new(transport, test_registry("mcp-server-a", "v1"));
        let error = surface
            .read_resource("mcp-server-a", "v1", "file:///docs/readme.md")
            .expect_err("text equal to the requested uri must be refused");
        assert!(error.contains("echoes the query arguments"), "{error}");
    }

    #[test]
    fn malformed_response_is_rejected() {
        let transport = fake_transport();
        transport.set_response(
            "mcp-server-a",
            McpResourceOperation::ListResources,
            Ok(McpTransportResponse {
                content: json!({ "resources": "not-an-array" }),
                truncated: false,
            }),
        );
        let surface = McpResourceSurface::new(transport, test_registry("mcp-server-a", "v1"));
        let error = surface
            .list_resources("mcp-server-a", "v1")
            .expect_err("malformed resources payload must fail closed");
        assert!(error.contains("`resources` array"), "{error}");

        let transport = fake_transport();
        transport.set_response(
            "mcp-server-a",
            McpResourceOperation::ReadResource,
            Ok(McpTransportResponse {
                content: json!({ "no_contents": true }),
                truncated: false,
            }),
        );
        let surface = McpResourceSurface::new(transport, test_registry("mcp-server-a", "v1"));
        let error = surface
            .read_resource("mcp-server-a", "v1", "file:///a")
            .expect_err("missing contents array must fail closed");
        assert!(error.contains("`contents` array"), "{error}");
    }

    #[test]
    fn content_item_uri_mismatch_is_rejected() {
        let transport = fake_transport();
        transport.set_response(
            "mcp-server-a",
            McpResourceOperation::ReadResource,
            Ok(McpTransportResponse {
                content: json!({
                    "contents": [
                        { "uri": "file:///other", "text": "unexpected resource" }
                    ]
                }),
                truncated: false,
            }),
        );
        let surface = McpResourceSurface::new(transport, test_registry("mcp-server-a", "v1"));
        let error = surface
            .read_resource("mcp-server-a", "v1", "file:///a")
            .expect_err("content uri must match the requested uri");
        assert!(error.contains("does not match the requested uri"), "{error}");
    }

    #[test]
    fn transport_error_propagates() {
        let transport = fake_transport();
        transport.set_response(
            "mcp-server-a",
            McpResourceOperation::ListResources,
            Err("transport disconnected".to_string()),
        );
        let surface = McpResourceSurface::new(transport, test_registry("mcp-server-a", "v1"));
        let error = surface
            .list_resources("mcp-server-a", "v1")
            .expect_err("transport errors must propagate");
        assert!(error.contains("transport disconnected"), "{error}");
    }
}
