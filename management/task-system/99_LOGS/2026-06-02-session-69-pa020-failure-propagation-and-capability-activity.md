# 2026-06-02 Session 69 - PA-020 Failure Propagation and Capability Activity

## Summary
- Fixed runtime capability failure propagation so MCP-backed tool execution no longer collapses `SourceUnavailable`, `PermissionDenied`, or `MalformedResponse` into `CapabilityNotFound`.
- Added capability activity metadata to the existing `TurnToolActivity` persistence chain.
- Exposed capability activity details in `ModelMonitorPage` drilldown without adding new Tauri commands.

## Code
- `src-tauri/src/agent/capability_bridge.rs`
  - `resolve_tool_call()` now returns normalized failure kinds instead of silently dropping them.
  - added `CapabilityFailureKind::as_str()`
  - added normalized failure-result builder for tool execution
- `src-tauri/src/agent/runtime.rs`
  - runtime now preserves resolve-time failure classes from capability execution
  - capability metadata is attached to parent `TurnToolActivity`
  - added runtime tests for `SourceUnavailable / PermissionDenied / MalformedResponse`
- `src-tauri/src/agent/telemetry.rs`
  - added `CapabilityInvocationRecord`
  - added optional `capability_invocation` field on `TurnToolActivity`
- `src-tauri/src/agent/session.rs`
  - session trace persistence roundtrip now covers capability activity metadata
- `src/types/runtime.ts`
  - added frontend types for `CapabilityFailureKind` and `CapabilityInvocationRecord`
- `src/stores/runtime.ts`
  - clone helpers now preserve nested capability activity metadata
- `src/components/ModelMonitorPage.vue`
  - added trace-level capability activity panel
- `tests/ModelMonitorPage.spec.ts`
  - added drilldown rendering coverage for capability activity

## Validation
- `cargo test --manifest-path src-tauri/Cargo.toml capability_bridge_propagates_ -- --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml file_backend_roundtrip_restores_turn_trace_history -- --nocapture`
- `cargo check --manifest-path src-tauri/Cargo.toml --lib`
- `cmd /c npm run test:unit -- --run tests/ModelMonitorPage.spec.ts`

## Remaining
- `5.2 / 6.3` still need capability-dimension summary aggregation at `ModelMonitorSummaryView` level.
- `3.4` still lacks unified `resource` fetch / `prompt_template` expansion result contracts.
- `6.1` still lacks an explicit planner/runtime boundary acceptance test.
