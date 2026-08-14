use crate::agent::tools::{
    ToolDefinitionContractView, ToolExposure, ToolOutcome, ToolRegistrySnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InvocationOrigin {
    Model,
    Child,
    Host,
    System,
}

/// Foundation-only dispatcher boundary. Its governed lifecycle is implemented in phase 3;
/// primitive and composite handlers must not acquire a `ToolRouter` directly.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDispatchRequest {
    pub origin: InvocationOrigin,
    pub descriptor_id: String,
    pub call_id: String,
    pub arguments: Value,
}

pub trait ToolDispatcher: Send + Sync {
    fn dispatch(&self, request: ToolDispatchRequest) -> ToolOutcome;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrimitiveToolHandlerRequest {
    pub descriptor_id: String,
    pub arguments: Value,
    /// Session injected from the dispatch context (authoritative for ownership decisions such as
    /// the Plan store's CrossSession guard). `None` when the dispatch context carries no session;
    /// handlers that need session ownership must fail closed rather than trust model-supplied
    /// keys (PA-076 phase-7 review P1-2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 会话级 workspace root（PA-080 P1-2 修复）：由 dispatch context 注入，工具权限判定
    /// 锚定在会话 workspace 而非进程 cwd。`None` 时 handler 回退构造时的默认 root。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
}

pub trait PrimitiveToolHandler: Send + Sync {
    fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String>;
}

pub trait RuntimeClock: Send + Sync {
    fn now_ms(&self) -> u64;
}

#[derive(Default)]
pub struct FakeClock {
    now_ms: Mutex<u64>,
}

impl FakeClock {
    pub fn new(now_ms: u64) -> Self {
        Self {
            now_ms: Mutex::new(now_ms),
        }
    }

    pub fn set_now_ms(&self, now_ms: u64) {
        *self.now_ms.lock().expect("fake clock mutex poisoned") = now_ms;
    }
}

impl RuntimeClock for FakeClock {
    fn now_ms(&self) -> u64 {
        *self.now_ms.lock().expect("fake clock mutex poisoned")
    }
}

/// Wall-clock `RuntimeClock` for production dispatchers. `now_ms` is Unix epoch milliseconds.
#[derive(Clone, Debug, Default)]
pub struct SystemClock;

impl RuntimeClock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingControlRequestKind {
    Interaction,
    Approval,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingControlRequestState {
    Pending,
    Consumed,
    Cancelled,
    Expired,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingControlRequest {
    pub request_id: String,
    pub request_kind: PendingControlRequestKind,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub turn_id: String,
    pub call_id: String,
    pub descriptor_snapshot_id: String,
    pub descriptor_id: String,
    pub final_args_digest: String,
    pub policy_digest: String,
    pub nonce: String,
    pub version: u64,
    pub expires_at_ms: u64,
    pub state: PendingControlRequestState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
}

impl PendingControlRequest {
    pub fn can_consume(
        &self,
        session_id: Option<&str>,
        expected_version: u64,
        nonce: &str,
        now_ms: u64,
    ) -> bool {
        self.state == PendingControlRequestState::Pending
            && self.session_id.as_deref() == session_id
            && self.version == expected_version
            && self.nonce == nonce
            && now_ms <= self.expires_at_ms
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnToolView {
    pub snapshot_id: String,
    #[serde(default)]
    pub direct_descriptor_ids: BTreeSet<String>,
    #[serde(default)]
    pub elevated_descriptor_ids: BTreeSet<String>,
}

impl TurnToolView {
    pub fn from_registry(snapshot: &ToolRegistrySnapshot) -> Self {
        let direct_descriptor_ids = snapshot
            .descriptors
            .iter()
            .filter(|descriptor| {
                descriptor.exposure == ToolExposure::ModelVisible
                    || descriptor.identity.primitive_name == "tool_search"
            })
            .map(|descriptor| descriptor.identity.descriptor_id.clone())
            .collect();
        Self {
            snapshot_id: snapshot.snapshot_id.clone(),
            direct_descriptor_ids,
            elevated_descriptor_ids: BTreeSet::new(),
        }
    }

    pub fn allows_model_descriptor(&self, descriptor_id: &str) -> bool {
        self.direct_descriptor_ids.contains(descriptor_id)
            || self.elevated_descriptor_ids.contains(descriptor_id)
    }

    pub fn elevate(&mut self, descriptor_id: impl Into<String>) {
        self.elevated_descriptor_ids.insert(descriptor_id.into());
    }

    pub fn elevate_from_registry(
        &mut self,
        snapshot: &ToolRegistrySnapshot,
        descriptor_id: &str,
    ) -> Result<(), String> {
        self.require_snapshot(snapshot)?;
        let descriptor = snapshot
            .descriptors
            .iter()
            .find(|descriptor| descriptor.identity.descriptor_id == descriptor_id)
            .ok_or_else(|| format!("unknown tool descriptor `{descriptor_id}`"))?;
        if descriptor.exposure != ToolExposure::Deferred {
            return Err(format!("tool descriptor `{descriptor_id}` is not deferred"));
        }
        self.elevate(descriptor_id);
        Ok(())
    }

    pub fn provider_contract_views(
        &self,
        snapshot: &ToolRegistrySnapshot,
    ) -> Result<Vec<ToolDefinitionContractView>, String> {
        self.require_snapshot(snapshot)?;
        Ok(snapshot
            .descriptors
            .iter()
            .filter(|descriptor| self.allows_model_descriptor(&descriptor.identity.descriptor_id))
            .map(|descriptor| descriptor.contract_view())
            .collect())
    }

    fn require_snapshot(&self, snapshot: &ToolRegistrySnapshot) -> Result<(), String> {
        if self.snapshot_id != snapshot.snapshot_id {
            return Err(format!(
                "turn tool view snapshot `{}` does not match registry snapshot `{}`",
                self.snapshot_id, snapshot.snapshot_id
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxAvailability {
    Available,
    Unavailable,
    HostApprovedUnsandboxed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxRequest {
    pub workspace_root: String,
    pub allow_network: bool,
    pub environment_allowlist: Vec<String>,
    /// When true the sandboxed child runs with a minimal environment: `env_clear` plus the
    /// essential vars plus `environment_allowlist`, never inheriting provider keys, session
    /// secrets, or ambient proxy env vars (design Decision 7, phase-5 review P1-1). An empty
    /// `environment_allowlist` still isolates — it keeps only the essential vars.
    pub isolate_environment: bool,
}

pub trait SandboxBackend: Send + Sync {
    fn availability(&self) -> SandboxAvailability;
    fn validate(&self, request: &SandboxRequest) -> Result<(), String>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStartRequest {
    pub session_id: String,
    pub program: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    pub sandbox: SandboxRequest,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Running,
    Exited,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessPollResult {
    pub state: ProcessState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

/// Process lifecycle port. A process group or Job Object implementation belongs behind this
/// port and must not be represented as a sandbox implementation.
pub trait ProcessBackend: Send + Sync {
    fn start(&self, request: &ProcessStartRequest) -> Result<String, String>;
    fn poll(&self, session_id: &str, handle: &str) -> Result<ProcessPollResult, String>;
    fn write_stdin(&self, session_id: &str, handle: &str, input: &[u8]) -> Result<(), String>;
    fn kill(&self, session_id: &str, handle: &str) -> Result<(), String>;
}

pub trait WebResolver: Send + Sync {
    fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String>;
}

#[derive(Default)]
pub struct FakeResolver {
    records: Mutex<BTreeMap<String, Result<Vec<IpAddr>, String>>>,
}

impl FakeResolver {
    pub fn set_addresses(&self, host: impl Into<String>, addresses: Vec<IpAddr>) {
        self.records
            .lock()
            .expect("fake resolver mutex poisoned")
            .insert(host.into(), Ok(addresses));
    }

    pub fn set_error(&self, host: impl Into<String>, error: impl Into<String>) {
        self.records
            .lock()
            .expect("fake resolver mutex poisoned")
            .insert(host.into(), Err(error.into()));
    }
}

impl WebResolver for FakeResolver {
    fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String> {
        self.records
            .lock()
            .expect("fake resolver mutex poisoned")
            .get(host)
            .cloned()
            .unwrap_or_else(|| Err(format!("no fake DNS record for `{host}`")))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpResourceOperation {
    ListResources,
    ListResourceTemplates,
    ReadResource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTransportRequest {
    pub source_id: String,
    pub source_revision: String,
    pub operation: McpResourceOperation,
    pub arguments: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTransportResponse {
    pub content: Value,
    #[serde(default)]
    pub truncated: bool,
}

pub trait McpTransport: Send + Sync {
    fn execute(&self, request: &McpTransportRequest) -> Result<McpTransportResponse, String>;
}

#[derive(Default)]
pub struct FakeMcpTransport {
    responses: Mutex<BTreeMap<(String, String), Result<McpTransportResponse, String>>>,
}

impl FakeMcpTransport {
    pub fn set_response(
        &self,
        source_id: impl Into<String>,
        operation: McpResourceOperation,
        response: Result<McpTransportResponse, String>,
    ) {
        self.responses
            .lock()
            .expect("fake MCP transport mutex poisoned")
            .insert(
                (source_id.into(), operation_key(&operation).to_string()),
                response,
            );
    }
}

impl McpTransport for FakeMcpTransport {
    fn execute(&self, request: &McpTransportRequest) -> Result<McpTransportResponse, String> {
        self.responses
            .lock()
            .expect("fake MCP transport mutex poisoned")
            .get(&(
                request.source_id.clone(),
                operation_key(&request.operation).to_string(),
            ))
            .cloned()
            .unwrap_or_else(|| Err(format!("no fake MCP response for `{}`", request.source_id)))
    }
}

fn operation_key(operation: &McpResourceOperation) -> &'static str {
    match operation {
        McpResourceOperation::ListResources => "list_resources",
        McpResourceOperation::ListResourceTemplates => "list_resource_templates",
        McpResourceOperation::ReadResource => "read_resource",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pending_request_requires_matching_session_version_nonce_and_expiry() {
        let request = PendingControlRequest {
            request_id: "request-1".to_string(),
            request_kind: PendingControlRequestKind::Interaction,
            session_id: Some("session-1".to_string()),
            run_id: Some("run-1".to_string()),
            turn_id: "turn-1".to_string(),
            call_id: "call-1".to_string(),
            descriptor_snapshot_id: "snapshot-1".to_string(),
            descriptor_id: "builtin:echo_input".to_string(),
            final_args_digest: "args".to_string(),
            policy_digest: "policy".to_string(),
            nonce: "nonce".to_string(),
            version: 3,
            expires_at_ms: 100,
            state: PendingControlRequestState::Pending,
            prompt: Some("continue?".to_string()),
            options: Some(json!(["yes", "no"])),
        };

        assert!(request.can_consume(Some("session-1"), 3, "nonce", 100));
        assert!(!request.can_consume(Some("session-2"), 3, "nonce", 100));
        assert!(!request.can_consume(Some("session-1"), 2, "nonce", 100));
        assert!(!request.can_consume(Some("session-1"), 3, "wrong", 100));
        assert!(!request.can_consume(Some("session-1"), 3, "nonce", 101));
    }

    #[test]
    fn turn_tool_view_only_allows_direct_or_elevated_descriptor_ids() {
        let mut view = TurnToolView {
            snapshot_id: "snapshot-1".to_string(),
            direct_descriptor_ids: BTreeSet::from(["builtin:workspace_read_file".to_string()]),
            elevated_descriptor_ids: BTreeSet::new(),
        };

        assert!(view.allows_model_descriptor("builtin:workspace_read_file"));
        assert!(!view.allows_model_descriptor("mcp:server:tool"));
        view.elevate("mcp:server:tool");
        assert!(view.allows_model_descriptor("mcp:server:tool"));
    }

    #[test]
    fn turn_tool_view_rejects_stale_snapshot_and_only_elevates_deferred_descriptors() {
        let registry = ToolRegistrySnapshot::builtin().expect("builtin registry should build");
        let mut view = TurnToolView::from_registry(&registry);

        assert!(view
            .elevate_from_registry(&registry, "builtin:workspace_path_info")
            .is_ok());
        assert!(view
            .elevate_from_registry(&registry, "builtin:workspace_read_file")
            .is_err());
        let contracts = view
            .provider_contract_views(&registry)
            .expect("matching snapshot should project provider contracts");
        assert!(contracts
            .iter()
            .any(|contract| contract.execution_primitive == "workspace_path_info"));

        let stale = ToolRegistrySnapshot::from_descriptors(
            "different-snapshot",
            registry.descriptors.clone(),
        )
        .expect("same descriptors with a new revision remain valid");
        assert!(view.provider_contract_views(&stale).is_err());
    }

    #[test]
    fn fake_runtime_harnesses_are_deterministic_and_source_bound() {
        let clock = FakeClock::new(10);
        assert_eq!(clock.now_ms(), 10);
        clock.set_now_ms(20);
        assert_eq!(clock.now_ms(), 20);

        let resolver = FakeResolver::default();
        resolver.set_addresses("public.example", vec!["203.0.113.10".parse().unwrap()]);
        assert_eq!(
            resolver
                .resolve("public.example")
                .expect("configured record"),
            vec!["203.0.113.10".parse::<IpAddr>().unwrap()]
        );
        assert!(resolver.resolve("missing.example").is_err());

        let transport = FakeMcpTransport::default();
        transport.set_response(
            "mcp-source",
            McpResourceOperation::ListResources,
            Ok(McpTransportResponse {
                content: json!({ "resources": [] }),
                truncated: false,
            }),
        );
        let response = transport
            .execute(&McpTransportRequest {
                source_id: "mcp-source".to_string(),
                source_revision: "v1".to_string(),
                operation: McpResourceOperation::ListResources,
                arguments: json!({}),
            })
            .expect("configured MCP response");
        assert_eq!(response.content, json!({ "resources": [] }));
        assert!(transport
            .execute(&McpTransportRequest {
                source_id: "other-source".to_string(),
                source_revision: "v1".to_string(),
                operation: McpResourceOperation::ListResources,
                arguments: json!({}),
            })
            .is_err());
    }
}
