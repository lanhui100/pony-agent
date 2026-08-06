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
use crate::agent::document_conversion::ReadDocumentHandler;
use crate::agent::image_artifact::ViewImageHandler;
use crate::agent::plan_state::{PlanControlHandler, PlanStore};
use crate::agent::tool_runtime::{
    InvocationOrigin, PrimitiveToolHandler, PrimitiveToolHandlerRequest, RuntimeClock, SystemClock,
};
use crate::agent::tools::{
    ToolCall, ToolRegistrySnapshot, ToolRouter, TOOL_PLAN_CONTROL, TOOL_WORKSPACE_READ_DOCUMENT,
    TOOL_VIEW_IMAGE,
};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// Read-only primitive surface shared by the migration gate and the governed composite authority.
/// Mirrors the legacy task-3.4 allowlist in `tools.rs` (kept in sync by contract).
const READ_ONLY_PRIMITIVES: &[&str] = &[
    "workspace_list_files",
    "workspace_read_file",
    "workspace_read_file_segment",
    "workspace_read_document",
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
    let router = Arc::new(ToolRouter::with_workspace_root(workspace.clone()));

    // ── Plan state control: session-owned, revisioned create/replace/merge/complete-step.
    // Each runtime owns one PlanStore; session isolation is enforced by the store's
    // CrossSession guard on every operation (PA-076 Decision 6). ──
    let plan_store = PlanStore::new();
    let plan_handler = Arc::new(PlanControlHandler::new(plan_store));

    // ── view_image: reference-based artifact with default include_bytes=false (design Decision 11).
    // Whether the encoded bytes reach a model is a provider modality choice. ──
    let view_image_handler = Arc::new(ViewImageHandler::new(workspace.clone()));

    // ── workspace_read_document: local office document → Markdown via anydoc, workspace-scoped,
    // with explicit output/input byte budgets and truncated evidence. ──
    let read_document_handler = Arc::new(ReadDocumentHandler::new(workspace.clone()));

    // Register every builtin primitive handler (composites are registered separately below).
    for descriptor in &registry.descriptors {
        let primitive = descriptor.identity.primitive_name.clone();
        if COMPOSITE_PRIMITIVES.contains(&primitive.as_str()) {
            continue;
        }
        // ── MCP resource read + ToolSearch boundary decision (PA-076 P2-6): these descriptors
        // stay in the capability registry execution path; their builtin entries are discovery
        // metadata only. Deliberately NOT registering governed handlers — doing so would create
        // a forbidden double execution path (governed dispatcher vs. runtime capability registry).
        // See tools.rs builtin_tools() comments and runtime/tool_exec.rs. ──
        if primitive == TOOL_PLAN_CONTROL {
            dispatcher.register_handler(
                descriptor.identity.descriptor_id.clone(),
                Arc::clone(&plan_handler) as Arc<dyn PrimitiveToolHandler>,
            );
            continue;
        }
        if primitive == TOOL_VIEW_IMAGE {
            dispatcher.register_handler(
                descriptor.identity.descriptor_id.clone(),
                Arc::clone(&view_image_handler) as Arc<dyn PrimitiveToolHandler>,
            );
            continue;
        }
        if primitive == TOOL_WORKSPACE_READ_DOCUMENT {
            dispatcher.register_handler(
                descriptor.identity.descriptor_id.clone(),
                Arc::clone(&read_document_handler) as Arc<dyn PrimitiveToolHandler>,
            );
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
    fn governed_executor_exposes_governed_dispatcher_through_trait_downcast() {
        let workspace = temp_workspace();
        let executor: Box<dyn ToolExecutor> =
            Box::new(build_governed_executor(Some(workspace)));
        // PA-076 P1-1: the runtime reaches the shared dispatcher via `ToolExecutor::as_any`.
        let governed = executor
            .as_any()
            .and_then(|any| any.downcast_ref::<GovernedToolExecutor>())
            .expect("default governed executor must downcast to GovernedToolExecutor");
        assert!(governed.dispatcher().registry().descriptors.len() > 0);
        // A non-governed executor exposes nothing (backward-compatible default).
        struct PlainExecutor;
        impl ToolExecutor for PlainExecutor {
            fn execute(&self, _call: &ToolCall) -> crate::agent::tools::ToolResult {
                crate::agent::tools::ToolResult {
                    tool_name: "plain".to_string(),
                    status: "ok".to_string(),
                    output: "{}".to_string(),
                    duration_ms: 0,
                }
            }
        }
        assert!(PlainExecutor.as_any().is_none());
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

    // ── PA-076 P2-6: plan_control via governed dispatch ──────────────────────────────────────

    #[test]
    fn governed_executor_dispatches_plan_control_create_replace_merge_and_complete_step() {
        let workspace = temp_workspace();
        let governed = build_governed_executor(Some(workspace.clone()));
        governed.set_context(crate::agent::dispatcher::DispatchContext {
            session_id: Some("session-1".to_string()),
            run_id: Some("run-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            ..Default::default()
        });

        // Create a draft plan with two steps.
        let created = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "create",
                "session_id": "session-1",
                "payload": {
                    "kind": "implement",
                    "summary": "Implement feature",
                    "steps": [
                        { "name": "draft", "summary": "Write spec" },
                        { "name": "code", "summary": "Implement" }
                    ]
                },
                "description": "test",
            }),
        ));
        assert_eq!(created.status, "ok", "create failed: {}", created.output);
        let created_value: Value =
            serde_json::from_str(&created.output).expect("plan output json");
        assert_eq!(created_value["planId"], "plan-1");
        assert_eq!(created_value["revision"], 1);
        assert_eq!(created_value["lifecycle"], "draft");
        assert_eq!(created_value["steps"].as_array().map(Vec::len), Some(2));

        // Merge a third step, preserving existing step ids.
        let merged = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "merge",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 1,
                "step": { "name": "test", "summary": "Add tests" },
                "description": "test",
            }),
        ));
        assert_eq!(merged.status, "ok", "merge failed: {}", merged.output);
        let merged_value: Value = serde_json::from_str(&merged.output).expect("plan output json");
        assert_eq!(merged_value["revision"], 2);
        assert_eq!(merged_value["steps"].as_array().map(Vec::len), Some(3));

        // Complete the first step → lifecycle advances to Executing.
        let step_id = merged_value["steps"][0]["stepId"]
            .as_str()
            .expect("step id")
            .to_string();
        let completed = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "complete_step",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 2,
                "step_id": step_id,
                "description": "test",
            }),
        ));
        assert_eq!(
            completed.status, "ok",
            "complete_step failed: {}",
            completed.output
        );
        let completed_value: Value =
            serde_json::from_str(&completed.output).expect("plan output json");
        assert_eq!(completed_value["steps"][0]["status"], "completed");
        assert_eq!(completed_value["lifecycle"], "executing");

        // Replace the plan content, keeping the same plan_id.
        let replaced = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "replace",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 3,
                "payload": { "kind": "refactor", "summary": "Refactor after review", "steps": [] },
                "description": "test",
            }),
        ));
        assert_eq!(
            replaced.status, "ok",
            "replace failed: {}",
            replaced.output
        );
        let replaced_value: Value =
            serde_json::from_str(&replaced.output).expect("plan output json");
        assert_eq!(replaced_value["planId"], "plan-1");
        assert_eq!(replaced_value["kind"], "refactor");
        assert_eq!(replaced_value["revision"], 4);
    }

    #[test]
    fn governed_executor_plan_control_stale_revision_and_missing_op_fail_closed() {
        let workspace = temp_workspace();
        let governed = build_governed_executor(Some(workspace.clone()));
        governed.set_context(crate::agent::dispatcher::DispatchContext {
            session_id: Some("session-1".to_string()),
            ..Default::default()
        });

        // Set up a plan so a stale revision can be attempted.
        let created = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "create",
                "session_id": "session-1",
                "payload": { "kind": "k", "summary": "s", "steps": [{ "name": "a", "summary": "sa" }] },
                "description": "test",
            }),
        ));
        assert_eq!(created.status, "ok", "plan setup failed: {}", created.output);

        let stale = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "replace",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 99,
                "payload": { "kind": "x", "summary": "y", "steps": [] },
                "description": "test",
            }),
        ));
        assert_eq!(stale.status, "error");
        assert!(stale.output.contains("stale_revision"), "{}", stale.output);

        // Missing `op` is rejected by schema validation.
        let no_op = governed.execute(&same_call(
            "Plan",
            json!({
                "session_id": "session-1",
                "description": "test",
            }),
        ));
        assert_eq!(no_op.status, "error");
        assert!(
            no_op.output.contains("missing required argument `op`"),
            "{}",
            no_op.output
        );
    }

    // ── PA-076 P2-6: view_image via governed dispatch ────────────────────────────────────────

    fn write_minimal_png(path: &std::path::Path, width: u32, height: u32) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(b"IEND");
        std::fs::write(path, bytes).expect("png fixture should write");
    }

    #[test]
    fn governed_executor_view_image_reference_artifact_default_omits_bytes() {
        let workspace = temp_workspace();
        write_minimal_png(&workspace.join("photo.png"), 320, 240);

        let governed = build_governed_executor(Some(workspace.clone()));
        let result = governed.execute(&same_call(
            "ViewImage",
            json!({ "path": "photo.png", "description": "test" }),
        ));
        assert_eq!(
            result.status, "ok",
            "view_image failed: {}",
            result.output
        );
        let value: Value =
            serde_json::from_str(&result.output).expect("view_image output json");
        assert_eq!(value["mimeType"], "image/png");
        assert_eq!(value["width"], 320);
        assert_eq!(value["height"], 240);
        assert_eq!(value["bytes"], Value::Null, "default include_bytes=false");
        assert_eq!(value["truncated"], false);
    }

    #[test]
    fn governed_executor_view_image_include_bytes_embeds_capped_payload() {
        let workspace = temp_workspace();
        write_minimal_png(&workspace.join("photo.png"), 64, 64);
        let full_len =
            std::fs::metadata(&workspace.join("photo.png")).unwrap().len();

        let governed = build_governed_executor(Some(workspace.clone()));
        // includeBytes=true with a small maxBytes → capped, truncated.
        let result = governed.execute(&same_call(
            "ViewImage",
            json!({
                "path": "photo.png",
                "includeBytes": true,
                "maxBytes": 16,
                "description": "test",
            }),
        ));
        assert_eq!(result.status, "ok", "capped view_image failed: {}", result.output);
        let value: Value =
            serde_json::from_str(&result.output).expect("view_image output json");
        assert_eq!(value["truncated"], true);
        assert_eq!(value["bytesLen"], full_len);
        // bytes are base64-encoded in the serialization; verify non-null.
        assert!(!value["bytes"].is_null());
    }

    #[test]
    fn governed_executor_view_image_missing_path_and_unknown_format_fail_closed() {
        let workspace = temp_workspace();
        let governed = build_governed_executor(Some(workspace.clone()));

        let missing = governed.execute(&same_call(
            "ViewImage",
            json!({ "description": "test" }),
        ));
        assert_eq!(missing.status, "error");
        assert!(
            missing.output.contains("missing required argument `path`"),
            "{}",
            missing.output
        );

        // A .txt file with valid PNG magic is rejected by extension check.
        std::fs::write(workspace.join("notes.txt"), b"not an image").expect("write");
        let unknown = governed.execute(&same_call(
            "ViewImage",
            json!({ "path": "notes.txt", "description": "test" }),
        ));
        assert_eq!(unknown.status, "error");
        assert!(
            unknown.output.contains("unsupported image extension"),
            "{}",
            unknown.output
        );
    }
}
