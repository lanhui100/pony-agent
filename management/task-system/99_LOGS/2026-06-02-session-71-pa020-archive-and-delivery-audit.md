# 2026-06-02 Session 71 - PA-020 Archive and Delivery Audit

## Summary
- Synced `PA-020` task-system status to completed state across dashboard/task board/task card.
- Added a runtime-generated capability activity acceptance test spanning `runtime -> session -> control_plane summary`.
- Fixed a missing frontend type import in `src/stores/runtime.ts`.
- Promoted the MCP capability bridge delta spec into the stable spec tree and archived the OpenSpec change.

## Code and Docs
- `src-tauri/src/agent/control_plane.rs`
  - added `load_model_monitor_summary_reads_capability_activity_from_runtime_generated_trace`
  - proves runtime-generated capability activity is persisted and aggregated into monitor summary
- `src/stores/runtime.ts`
  - added missing `BuildContextObservation` type import so `vue-tsc` can pass
- `openspec/specs/mcp-capability-bridge/spec.md`
  - added stable spec copy for the completed MCP capability bridge
- `openspec/changes/archive/2026-06-02-add-mcp-capability-bridge/`
  - archived completed change directory after stable spec sync
- `management/task-system/00_DASHBOARD.md`
  - updated `PA-020` status to completed
- `management/task-system/01_TASK_BOARD.md`
  - moved `PA-020` from `In Progress` to `Done`
- `management/task-system/03_TASKS/PA-020-build-mcp-capability-bridge.md`
  - added final closeout notes and archive/build verification record

## Validation
- `cargo test --manifest-path src-tauri/Cargo.toml load_model_monitor_summary_reads_capability_activity_from_runtime_generated_trace -- --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml load_model_monitor_summary_aggregates_capability_usage_dimensions -- --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml capability_bridge_keeps_mcp_as_runtime_ingress_not_planner_scheduler_state -- --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml registry_builds_normalized_resource_fetch_results -- --nocapture`
- `cargo test --manifest-path src-tauri/Cargo.toml registry_builds_normalized_prompt_expansion_results -- --nocapture`
- `cmd /c npm run test:unit -- --run tests/ModelMonitorPage.spec.ts`
- `cmd /c npm run build`

## Notes
- `resource / prompt_template` remains contract-first in this change. The normalized result/failure contract is implemented and tested, but no separate runtime fetch/expand execution entry was introduced in `PA-020`.
- Rust test output still contains existing `dead_code` warnings around contract-only capability structures, but the targeted verification suite passed.
