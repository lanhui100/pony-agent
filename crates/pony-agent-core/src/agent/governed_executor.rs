//! Production seam for adopting the governed dispatcher as the runtime's tool-execution engine
//! (PA-076 runtime-switch milestone).
//!
//! `build_governed_executor` constructs a `GovernedToolExecutor` over the full builtin tool
//! surface: every builtin primitive is bridged to its legacy `ToolRouter` implementation, and the
//! governed `workspace_batch` / `workspace_gather_context` composites are registered with the same
//! read-only child authority as the legacy migration gate. The runtime can switch its tool engine
//! from a bare `ToolRouter` to this executor without changing its own surface.
//!
//! Migration notes (deliberate, documented):
//! - A `LegacyCompatiblePolicyEvaluator` reproduces the legacy runtime's direct-execution
//!   behavior (every builtin allowed; the legacy `ToolRouter` applied its own tool-level risk
//!   checks). The dispatcher's conservative default is intentionally overridden here so the switch
//!   does not change today's tool-visible behavior. Real approval semantics land with the
//!   permission-contract work (phase 4 / task 3.6 closeout).
//! - Execute-scope tools (`workspace_run_command`) fail closed with `sandbox_unavailable` until a
//!   real `SandboxBackend` is registered — the designed behavior (design.md Decision 7); the
//!   process-lifecycle work (phase 5) supplies that backend.
//! - Legacy tool error codes that are not in the dispatcher's known-code set surface as
//!   `handler_error`; status and message fidelity are preserved.

use crate::agent::budget::DispatchBudgetConfig;
use crate::agent::dispatcher::{
    GovernedDispatcher, PermissionDecision, PermissionVerdict, ToolPolicyEvaluator,
};
use crate::agent::dispatcher_composites::{
    register_governed_composites, GovernedToolExecutor,
};
use crate::agent::tool_runtime::{
    InvocationOrigin, PrimitiveToolHandler, PrimitiveToolHandlerRequest, RuntimeClock, SystemClock,
};
use crate::agent::tools::{ToolCall, ToolRegistrySnapshot, ToolRouter};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// Read-only primitive surface shared by the migration gate and the governed composite authority.
/// Mirrors the legacy task-3.4 allowlist in `tools.rs` (kept in sync by contract).
const READ_ONLY_PRIMITIVES: &[&str] = &[
    "workspace_list_files",
    "workspace_read_file",
    "workspace_read_file_segment",
    "workspace_path_info",
    "workspace_search_text",
    "workspace_glob_files",
];

const COMPOSITE_PRIMITIVES: &[&str] = &["workspace_batch", "workspace_gather_context"];

/// Migration policy evaluator that reproduces the legacy runtime's direct-execution behavior.
/// The dispatcher's conservative write/execute default is intentionally overridden so adopting the
/// governed engine does not change tool-visible behavior; real approval semantics replace this at
/// the permission-contract milestone.
#[derive(Clone, Debug, Default)]
pub struct LegacyCompatiblePolicyEvaluator;

impl ToolPolicyEvaluator for LegacyCompatiblePolicyEvaluator {
    fn evaluate(
        &self,
        descriptor: &crate::agent::tools::ToolDescriptor,
        _origin: &InvocationOrigin,
        _final_arguments: &Value,
    ) -> PermissionDecision {
        // The Ask product tool is host-mediated in the governed path: dispatching it persists an
        // `Interaction` `PendingControlRequest` (WaitingHost) instead of echoing, which is the
        // design Decision 5 behavior that replaces the legacy echo placeholder (P1-1 wiring).
        let is_ask = descriptor.identity.model_name == "Ask"
            || descriptor.identity.primitive_name == "echo_input";
        if is_ask {
            PermissionDecision {
                verdict: PermissionVerdict::WaitingHost,
                decision_source: "ask_descriptor".to_string(),
                reason: None,
            }
        } else {
            PermissionDecision {
                verdict: PermissionVerdict::Allow,
                decision_source: "legacy_compatible".to_string(),
                reason: None,
            }
        }
    }
}

/// Bridges one builtin primitive to the legacy `ToolRouter` implementation, preserving the exact
/// `ToolResult.output` byte-for-byte on success (returned as a string `Value` so the dispatcher's
/// `render_output` passes it through unchanged).
struct RouterPrimitiveHandler {
    router: Arc<ToolRouter>,
    primitive: String,
}

impl PrimitiveToolHandler for RouterPrimitiveHandler {
    fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        let result = self.router.execute(&ToolCall {
            call_id: None,
            name: self.primitive.clone(),
            arguments: request.arguments.clone(),
            plan: None,
        });
        if result.status == "ok" {
            Ok(Value::String(result.output))
        } else {
            Err(legacy_error_message(&result.output))
        }
    }
}

/// Build a fail-closed `code: message` handler error from a legacy `ToolResult.output`, preferring
/// the structured `error.code`/`error.message` when present so the dispatcher's `handler_failure`
/// can lift a known code.
fn legacy_error_message(output: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(output) {
        if let Some(error) = value.get("error") {
            let code = error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("handler_error");
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(output);
            return format!("{code}: {message}");
        }
    }
    format!("handler_error: {output}")
}

/// Build a governed executor over the full builtin tool surface, ready to be adopted as the
/// runtime's tool-execution engine. `workspace_root` seeds the `ToolRouter` implementations and the
/// dispatcher's workspace facts.
pub fn build_governed_executor(workspace_root: Option<PathBuf>) -> GovernedToolExecutor {
    let registry = Arc::new(
        ToolRegistrySnapshot::builtin().expect("builtin registry must validate"),
    );
    let clock: Arc<dyn RuntimeClock> = Arc::new(SystemClock);
    let dispatcher = GovernedDispatcher::new(registry.clone(), clock);
    dispatcher.register_policy_evaluator(Arc::new(LegacyCompatiblePolicyEvaluator));
    // Legacy-compatible budgets: the switch must reproduce the legacy ToolRouter's lack of
    // tool-level output-byte/deadline caps (phase-4..7 review P1-2), not silently regress large
    // reads/batches. The governed path still enforces cancellation + child-call budgets.
    dispatcher.set_budget_config(DispatchBudgetConfig {
        unbounded_output_and_deadline: true,
        ..Default::default()
    });

    let workspace = workspace_root.unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    });
    let router = Arc::new(ToolRouter::with_workspace_root(workspace));

    // Register every builtin primitive handler (composites are registered separately below).
    for descriptor in &registry.descriptors {
        let primitive = descriptor.identity.primitive_name.clone();
        if COMPOSITE_PRIMITIVES.contains(&primitive.as_str()) {
            continue;
        }
        dispatcher.register_handler(
            descriptor.identity.descriptor_id.clone(),
            Arc::new(RouterPrimitiveHandler {
                router: Arc::clone(&router),
                primitive,
            }),
        );
    }

    // Register the governed composites with the read-only child authority matching the legacy gate.
    let authority = READ_ONLY_PRIMITIVES
        .iter()
        .map(|primitive| format!("builtin:{primitive}"))
        .collect::<Vec<_>>();
    register_governed_composites(&dispatcher, authority.clone(), authority);

    GovernedToolExecutor::new(dispatcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tool_runtime::PendingControlRequestKind;
    use crate::agent::tools::ToolExecutor;
    use serde_json::json;
    use std::fs;

    static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_workspace() -> PathBuf {
        // Unique per call so parallel tests in one process never share/race a fixture directory.
        let seq = TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pony-governed-executor-test-{}-{seq}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).expect("create test workspace");
        root
    }

    fn same_call(name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            call_id: None,
            name: name.to_string(),
            arguments,
            plan: None,
        }
    }

    #[test]
    fn governed_executor_matches_legacy_router_output_for_model_visible_tools() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.rs"), "fn main() {}\n").expect("write fixture");
        fs::write(workspace.join("sub/mod.rs"), "pub fn helper() {}\n").expect("write fixture");

        let legacy = ToolRouter::with_workspace_root(workspace.clone());
        let governed = build_governed_executor(Some(workspace.clone()));

        // The model-visible product surface (what the runtime actually dispatches): List, Search,
        // Glob. Read/Write/Edit are covered separately; Run is sandbox-gated; Ask is host-mediated
        // (covered by its own assertion below — no longer a legacy echo).
        let cases = vec![
            same_call("List", json!({ "path": ".", "description": "test" })),
            same_call(
                "Search",
                json!({ "query": "fn", "path": ".", "description": "test" }),
            ),
            same_call("Glob", json!({ "pattern": "**/*.rs", "description": "test" })),
        ];

        for call in cases {
            let legacy_result = legacy.execute(&call);
            let governed_result = governed.execute(&call);
            assert_eq!(
                governed_result.status, legacy_result.status,
                "status mismatch for `{}`: {}",
                call.name,
                governed_result.output
            );
            // Search/Glob outputs now carry `durationMs` (non-deterministic wall time); strip it
            // before the byte-for-byte comparison so only deterministic fields are compared.
            assert_eq!(
                normalize_output(&governed_result.output),
                normalize_output(&legacy_result.output),
                "output mismatch for `{}`",
                call.name
            );
        }
    }

    #[test]
    fn governed_executor_ask_is_host_mediated_and_persists_the_question() {
        let workspace = temp_workspace();
        let governed = build_governed_executor(Some(workspace));
        governed.set_context(crate::agent::dispatcher::DispatchContext {
            session_id: Some("session-ask".to_string()),
            run_id: Some("run-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            ..Default::default()
        });
        let result = governed.execute(&same_call(
            "Ask",
            json!({ "text": "继续吗？", "description": "test" }),
        ));
        // Pending control outcome surfaces as the legacy `control_outcome_pending` marker, which
        // the runtime detects to suspend the run (design Decision 5, P1-1 wiring).
        assert_eq!(result.status, "error");
        assert!(
            result.output.contains("control_outcome_pending"),
            "expected pending control marker, got: {}",
            result.output
        );
        // The persisted request is an Interaction kind bound to the real session, with the
        // model's question surfaced verbatim (P2-9).
        let pending = governed.dispatcher().pending_requests();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request_kind, PendingControlRequestKind::Interaction);
        assert_eq!(pending[0].session_id.as_deref(), Some("session-ask"));
        assert_eq!(pending[0].turn_id, "turn-1");
        assert_eq!(pending[0].prompt.as_deref(), Some("继续吗？"));
    }

    /// Parse a tool output and drop the non-deterministic `durationMs` field for fidelity
    /// comparisons.
    fn normalize_output(output: &str) -> Value {
        if let Ok(mut value) = serde_json::from_str::<Value>(output) {
            if let Some(object) = value.as_object_mut() {
                object.remove("durationMs");
            }
            value
        } else {
            Value::String(output.to_string())
        }
    }

    #[test]
    fn governed_executor_matches_legacy_router_for_write_and_edit() {
        let workspace = temp_workspace();
        let legacy = ToolRouter::with_workspace_root(workspace.clone());
        let governed = build_governed_executor(Some(workspace.clone()));

        let write = same_call(
            "Write",
            json!({ "path": "out.txt", "content": "hello\n", "description": "test" }),
        );
        // Reset the file before each run so `overwroteExisting` is identical.
        fs::remove_file(workspace.join("out.txt")).ok();
        let legacy_write = legacy.execute(&write);
        assert_eq!(legacy_write.status, "ok");
        fs::remove_file(workspace.join("out.txt")).ok();
        let governed_write = governed.execute(&write.clone());
        assert_eq!(
            governed_write.status, "ok",
            "governed write failed: {}",
            governed_write.output
        );
        assert_eq!(governed_write.output, legacy_write.output);

        let edit = same_call(
            "Edit",
            json!({
                "path": "out.txt",
                "oldText": "hello",
                "newText": "world",
                "description": "test",
            }),
        );
        // Re-create the fixture before each edit run (edit requires the target to exist).
        fs::write(workspace.join("out.txt"), "hello\n").expect("write fixture");
        let legacy_edit = legacy.execute(&edit);
        assert_eq!(
            legacy_edit.status, "ok",
            "legacy edit failed: {}",
            legacy_edit.output
        );
        fs::write(workspace.join("out.txt"), "hello\n").expect("write fixture");
        let governed_edit = governed.execute(&edit.clone());
        assert_eq!(
            governed_edit.status, "ok",
            "governed edit failed: {}",
            governed_edit.output
        );
        assert_eq!(governed_edit.output, legacy_edit.output);
    }

    #[test]
    fn governed_executor_fails_closed_for_run_without_sandbox_backend() {
        let workspace = temp_workspace();
        let governed = build_governed_executor(Some(workspace));
        let result = governed.execute(&same_call(
            "Run",
            json!({ "command": "echo hi", "description": "test" }),
        ));
        // Execute scope + no sandbox backend => designed fail-closed (design.md Decision 7);
        // phase 5 supplies the real SandboxBackend.
        assert_eq!(result.status, "error");
        assert!(
            result.output.contains("sandbox_unavailable"),
            "expected sandbox_unavailable, got: {}",
            result.output
        );
    }

    #[test]
    fn governed_executor_runs_read_only_batch_children_through_child_dispatch() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.rs"), "fn main() {}\n").expect("write fixture");
        let governed = build_governed_executor(Some(workspace));

        let result = governed.execute(&same_call(
            "BatchExecute",
            json!({
                "calls": [
                    { "name": "workspace_read_file", "arguments": { "path": "demo.rs", "description": "test" } },
                    { "name": "workspace_path_info", "arguments": { "path": "demo.rs", "description": "test" } },
                ],
                "description": "test",
            }),
        ));
        assert_eq!(result.status, "ok");
        let payload: Value = serde_json::from_str(&result.output).expect("batch output json");
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["successCount"], 2);
        assert_eq!(payload["results"].as_array().map(Vec::len), Some(2));
    }
}
