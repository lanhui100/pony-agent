# 2026-06-02 Session 70 - PA-020 Capability Summary and Boundary Acceptance

## Summary
- Added capability-dimension summary aggregation to the model monitor read model.
- Exposed capability source, invocation mode, and failure class summaries in `ModelMonitorPage`.
- Added an acceptance test proving MCP remains a runtime capability-ingress layer rather than planner scheduler state.

## Code
- `src-tauri/src/agent/control_plane.rs`
  - added `ModelMonitorActivityRow`
  - extended `ModelMonitorSummaryView` with `capability_sources`, `capability_invocation_modes`, and `capability_failure_classes`
  - aggregated capability activity from persisted `TurnToolActivity.capability_invocation`
  - added summary aggregation test for capability usage dimensions
- `src-tauri/src/agent/runtime.rs`
  - added planner/runtime boundary acceptance test covering normalized `ToolCall` handoff into capability bridge resolve
- `src/types/runtime.ts`
  - added `ModelMonitorActivityRow`
  - extended `ModelMonitorSummaryView` with capability summary arrays
- `src/components/ModelMonitorPage.vue`
  - added capability summary sections for sources, invocation modes, and failure classes
- `tests/ModelMonitorPage.spec.ts`
  - extended summary fixture and assertions for the new capability summary sections
- `openspec/changes/add-mcp-capability-bridge/tasks.md`
  - marked `5.2`, `6.1`, and `6.3` complete

## Validation
- `cargo test --manifest-path src-tauri/Cargo.toml load_model_monitor_summary_aggregates_capability_usage_dimensions -- --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml capability_bridge_keeps_mcp_as_runtime_ingress_not_planner_scheduler_state -- --nocapture`
- `cmd /c npm run test:unit -- --run tests/ModelMonitorPage.spec.ts`

## Remaining
- `3.4` has now been closed in code via normalized `resource` fetch / `prompt_template` expansion result contracts and tests.
- `4.3` still needs the planner consumption rule fully closed in the spec/design artifacts.

## Superseded
- Session 71 completed the remaining closeout work: runtime-generated capability activity end-to-end verification, stable spec sync, task-system sync, and archive of `add-mcp-capability-bridge`.
